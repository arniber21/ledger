//! Validation for the generated Verdict message and its assurance envelope.
//!
//! A Verdict is the only thing Sentinel publishes about a Specification, and ADR-0017
//! makes its authority — authoritative, provisional or inconclusive — a claim about the
//! evidence behind it rather than a confidence score. ADR-0022 supplies the source
//! assurance tiers that decide whether the evidence can anchor an authoritative
//! conclusion, and ADR-0021 the settlement basis an authoritative outcome must name.
//! Everything here exists to reject an envelope whose stated authority its evidence
//! cannot support. [`Verdict`] is the validated newtype; `TryFrom<raw_model::Verdict>`
//! is the only way to construct one.
//!
//! Inside, `AssuranceValidator` is the seam. `TryFrom` checks the Verdict header -- the
//! identifier, the outcome and the inconclusive reason -- and hands the Assurance
//! Envelope to `AssuranceValidator::check`, which owns every rule that reads the
//! envelope, including the Tier Settlement and Late Data Policy claims in its own header.
//! Its constructor collects and checks the evidence in one straight-line pass and returns
//! a value whose collections are, by construction, coherent; the outcome-basis rules are
//! methods on that value, so they read the evidence rather than being handed it. That is
//! why the type exists: the rules deciding whether an Anchor Source really anchored a
//! given witness are the deepest here and were the ones threading the most state between
//! functions.
//!
//! Canonicalization is a separate concern and runs only after validation has succeeded.
//! The order is load-bearing in a way that is easy to miss: validation reports the first
//! failing condition it finds, and some of those conditions are checked in wire order --
//! the witness events of a Settlement Basis are validated as they arrive and sorted only
//! afterwards. Canonicalizing first would silently change which error a multi-defect
//! envelope reports. `assurance_validation_reports_the_first_failing_condition` pins the
//! order the checks run in; nothing pins the order the input arrives in.
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::missing_panics_doc)]

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::{self, Display, Formatter};

use prost::Message;

use crate::canonical::source_from_wire;
use crate::wire::model::v1 as raw_model;
use crate::wire_helpers::{
    compare_optional_sources, owned_source_key, sort_sources, sort_watermarks, source_key,
    validate_source, validate_watermarks, SourceKey, WireError,
};
use crate::{
    EventId, FilterEpoch, SourceCapabilities, SourceCapability, SourceIdentity,
    BARRIER_PENDING_DETAIL_PREFIX,
};

/// Platform default raw-Event retention days (ADR-0026 §6).
///
/// Core construction sites hardcode this under #206 disposition A until Tenant-aware
/// injection exists. [`Verdict::decode`] also uses it as historical codec recovery for
/// pre-#206 payloads — that is not an export default for new construction.
pub const PLATFORM_DEFAULT_RAW_EVENT_RETENTION_DAYS: u32 = 30;

/// Inclusive lower bound for raw-Event retention days (ADR-0026 §6).
pub const RAW_EVENT_RETENTION_DAYS_MIN: u32 = 7;

/// Inclusive upper bound for raw-Event retention days (ADR-0026 §6).
pub const RAW_EVENT_RETENTION_DAYS_MAX: u32 = 365;

/// Inclusive lower bound for evidence-extract retention days (#243 / ADR-0026 addendum).
///
/// Preserves the #206 wire floor (`≥ 1`). Values in `1..=6` remain legal; the analogy to
/// raw-Event `7..=365` is shape only (validated inclusive range), not identical endpoints.
pub const EVIDENCE_EXTRACT_RETENTION_DAYS_MIN: u32 = 1;

/// Inclusive upper bound for evidence-extract retention days (#243 / ADR-0026 addendum).
///
/// Absolute platform ceiling. **Not** coupled to [`RAW_EVENT_RETENTION_DAYS_MAX`] on the
/// same envelope — evidence-extract plaintext may outlive redacted/aged raw Events
/// (ADR-0026 §8). A one-year Investigator binding window is a confidentiality residual
/// relative to §8's “short-lived” framing; store reap is a later ticket.
pub const EVIDENCE_EXTRACT_RETENTION_DAYS_MAX: u32 = 365;

/// Tenant-config / store-enabled default for configured evidence-extract retention days
/// (#243 / ADR-0026 §6 / §8).
///
/// Use only when the configured arm is chosen (Tenant configuration surfaces, future
/// extract-store enablement). **Not** a Core construction default while the store is
/// undeployed, and **not** a [`Verdict::decode`] backfill value — those continue to emit
/// / recover [`RetentionWindow::NotDeployed`].
pub const DEFAULT_CONFIGURED_EVIDENCE_EXTRACT_RETENTION_DAYS: u32 = 90;

/// Validated raw-Event retention horizon in days.
///
/// Constructible only inside `7..=365`. Sentinel Events are not the EU AI Act Art. 12
/// automatically generated log; the platform default of
/// [`PLATFORM_DEFAULT_RAW_EVENT_RETENTION_DAYS`] sits below the six-month Art. 19 / 26(6)
/// deployer floor on purpose and must not be silently raised to fake that floor.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RawEventRetentionDays(u32);

impl RawEventRetentionDays {
    /// Creates a raw-Event retention horizon, or rejects an out-of-bounds value.
    ///
    /// # Errors
    ///
    /// Returns [`VerdictError::InvalidRange`] when `days` is outside `7..=365`.
    pub fn try_new(days: u32) -> Result<Self, VerdictError> {
        if (RAW_EVENT_RETENTION_DAYS_MIN..=RAW_EVENT_RETENTION_DAYS_MAX).contains(&days) {
            Ok(Self(days))
        } else {
            Err(VerdictError::InvalidRange(
                "assurance.raw_event_retention_days",
            ))
        }
    }

    /// Returns the platform default horizon (30 days).
    #[must_use]
    pub const fn platform_default() -> Self {
        Self(PLATFORM_DEFAULT_RAW_EVENT_RETENTION_DAYS)
    }

    /// Returns the underlying day count.
    #[must_use]
    pub const fn value(self) -> u32 {
        self.0
    }
}

/// Validated evidence-extract retention horizon in days (#243).
///
/// Constructible only inside `1..=365`. There is **no** `platform_default()` — Core and
/// legacy decode continue to claim [`RetentionWindow::NotDeployed`] while the extract
/// store is undeployed. [`Self::configured_default`] is the Tenant-config / store-enabled
/// 90-day default only.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct EvidenceExtractRetentionDays(u32);

impl EvidenceExtractRetentionDays {
    /// Creates an evidence-extract retention horizon, or rejects an out-of-bounds value.
    ///
    /// # Errors
    ///
    /// Returns [`VerdictError::InvalidRange`] when `days` is outside `1..=365`.
    pub fn try_new(days: u32) -> Result<Self, VerdictError> {
        if (EVIDENCE_EXTRACT_RETENTION_DAYS_MIN..=EVIDENCE_EXTRACT_RETENTION_DAYS_MAX)
            .contains(&days)
        {
            Ok(Self(days))
        } else {
            Err(VerdictError::InvalidRange(
                "assurance.evidence_extract_retention_days",
            ))
        }
    }

    /// Returns the Tenant-config / store-enabled default horizon (90 days).
    ///
    /// **Not** for Core envelope construction while the evidence-extract store is
    /// undeployed, and **not** a decode backfill value.
    #[must_use]
    pub const fn configured_default() -> Self {
        Self(DEFAULT_CONFIGURED_EVIDENCE_EXTRACT_RETENTION_DAYS)
    }

    /// Returns the underlying day count.
    #[must_use]
    pub const fn value(self) -> u32 {
        self.0
    }
}

/// Evidence-extract retention window claimed on the Assurance Envelope (ADR-0026 §6 / §8).
///
/// [`Self::NotDeployed`] is the truthful claim when this deployment operates no
/// evidence-extract store. Absence of the wire oneof is a defect, not this state.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum RetentionWindow {
    /// Configured retention horizon in days (`1..=365`; see [`EvidenceExtractRetentionDays`]).
    Days(EvidenceExtractRetentionDays),
    /// This deployment operates no evidence-extract store.
    NotDeployed,
}

/// Redaction policy claimed on the Assurance Envelope (ADR-0026 §7).
///
/// [`Self::NoPolicyConfigured`] is the truthful claim when this deployment has no
/// redaction policy. Absence of the wire oneof is a defect, not this state. The
/// forecast-at-settle path inventory lives on [`Verdict::redacted_attributes`], not
/// on this version claim.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum RedactionPolicyVersion {
    /// Opaque non-empty policy version string in effect at settlement.
    Version(String),
    /// This deployment has no redaction policy configured.
    NoPolicyConfigured,
}

impl RedactionPolicyVersion {
    /// Returns the configured version string, when present.
    #[must_use]
    pub fn as_version(&self) -> Option<&str> {
        match self {
            Self::Version(version) => Some(version.as_str()),
            Self::NoPolicyConfigured => None,
        }
    }
}

/// Forecast-at-settle inventory entry claiming an attribute path for post-settle
/// redaction (ADR-0026 §7; issue #244).
///
/// `attribute_path` is an opaque Tenant-policy key — not a parsed redaction DSL.
/// Value hashes stay on the ADR-0026 §8 / evidence-extract surface.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub struct RedactedAttributeRef {
    /// Event that carries the claimed attribute.
    pub event_id: String,
    /// Opaque Tenant-policy attribute path key.
    pub attribute_path: String,
}

/// Validation failure at the Verdict boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum VerdictError {
    /// Required scalar text is empty.
    MissingScalar(&'static str),
    /// Required message is absent.
    MissingField(&'static str),
    /// Enum number is unknown.
    UnknownEnum {
        /// Enum field name.
        field: &'static str,
        /// Unrecognized numeric value.
        value: i32,
    },
    /// Enum uses its invalid zero sentinel.
    UnspecifiedEnum(&'static str),
    /// A set-like repeated value is duplicated.
    DuplicateValue(&'static str),
    /// Verdict outcome and inconclusive reason disagree.
    VerdictReasonMismatch,
    /// An authoritative Verdict has an unsettled tier.
    AuthoritativeTierUnsettled,
    /// A provisional Verdict does not represent a settled hot path awaiting cold-path work
    /// or a pending activation barrier.
    ProvisionalTierMismatch,
    /// An authoritative Verdict claims a pending activation barrier.
    AuthoritativeBarrierPending,
    /// A cleared barrier is missing its target, a required-source row, an acknowledgement,
    /// or an acknowledgement at or above the target.
    ClearedBarrierIncomplete,
    /// An undeclared barrier carried a Specification-level target Filter epoch.
    UndeclaredBarrierHasTarget,
    /// Source-keyed assurance evidence refers outside the required source set.
    AssuranceSourceMismatch(&'static str),
    /// An authoritative envelope does not cover every required source.
    AuthoritativeCoverageIncomplete(&'static str),
    /// A source-tier record claims anchor evidence that its tier cannot provide.
    InvalidAnchorEvidence,
    /// A source-tier downgrade is missing or disagrees with the contributing sources.
    SourceTierGapMismatch,
    /// An authoritative conclusion has no valid anchor-tier evidence.
    MissingAnchorEvidence,
    /// An authoritative conclusion omitted its outcome-specific settlement basis.
    MissingSettlementBasis,
    /// The settlement-basis kind cannot justify the Verdict outcome or authority.
    SettlementBasisMismatch,
    /// A complete evidence basis contains an open condition that can change the outcome.
    AuthoritativeEvidenceIncomplete(&'static str),
    /// A settlement Watermark is ahead of the evidence Watermark for its source.
    SettlementEvidenceIncomplete(&'static str),
    /// A decisive violation witness is malformed or not closed over its evidence.
    InvalidDecisiveWitness(&'static str),
    /// A decisive proof did not enumerate exactly the open outcome-invariant evidence.
    InvariantLimitationsMismatch(&'static str),
    /// Tier settlement and settlement Watermarks disagree.
    SettlementWatermarkMismatch(&'static str),
    /// Nested canonical-model evidence is invalid.
    InvalidEvidence(crate::EventError),
    /// A numeric evidence interval is inverted.
    InvalidRange(&'static str),
    /// Non-empty `redacted_attributes` under `redaction_policy` None (phantom inventory).
    RedactedAttributesWithoutPolicy,
    /// The bytes did not contain a valid Protobuf message.
    Decode(String),
}

impl Display for VerdictError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingScalar(field) => write!(formatter, "required scalar is missing: {field}"),
            Self::MissingField(field) => write!(formatter, "required message is missing: {field}"),
            Self::UnknownEnum { field, value } => {
                write!(formatter, "unknown {field} enum value: {value}")
            }
            Self::UnspecifiedEnum(field) => write!(formatter, "{field} must not be unspecified"),
            Self::DuplicateValue(field) => {
                write!(formatter, "duplicate value in set-like field: {field}")
            }
            Self::VerdictReasonMismatch => {
                formatter.write_str("Verdict outcome and inconclusive reason disagree")
            }
            Self::AuthoritativeTierUnsettled => {
                formatter.write_str("authoritative Verdict requires all required tiers to settle")
            }
            Self::ProvisionalTierMismatch => formatter.write_str(
                "provisional Verdict requires a settled hot path and either a pending cold path or a pending activation barrier",
            ),
            Self::AuthoritativeBarrierPending => formatter
                .write_str("authoritative Verdict cannot carry a pending activation barrier"),
            Self::ClearedBarrierIncomplete => formatter.write_str(
                "cleared activation barrier requires a target Filter epoch and an acknowledged epoch at or above it on every required source",
            ),
            Self::UndeclaredBarrierHasTarget => formatter.write_str(
                "undeclared activation barrier must not carry a target Filter epoch",
            ),
            Self::AssuranceSourceMismatch(field) => {
                write!(formatter, "assurance source is not required: {field}")
            }
            Self::AuthoritativeCoverageIncomplete(field) => {
                write!(
                    formatter,
                    "authoritative assurance lacks required-source coverage: {field}"
                )
            }
            Self::InvalidAnchorEvidence => {
                formatter.write_str("source tier and anchored Event evidence disagree")
            }
            Self::SourceTierGapMismatch => {
                formatter.write_str("source-tier gap does not match contributing-only evidence")
            }
            Self::MissingAnchorEvidence => {
                formatter.write_str("authoritative conclusion requires valid anchor-tier evidence")
            }
            Self::MissingSettlementBasis => {
                formatter.write_str("authoritative conclusion requires an outcome settlement basis")
            }
            Self::SettlementBasisMismatch => formatter
                .write_str("settlement basis does not justify Verdict outcome or authority"),
            Self::AuthoritativeEvidenceIncomplete(field) => {
                write!(formatter, "authoritative evidence remains open: {field}")
            }
            Self::SettlementEvidenceIncomplete(field) => {
                write!(
                    formatter,
                    "evidence does not reach settlement Watermark: {field}"
                )
            }
            Self::InvalidDecisiveWitness(field) => {
                write!(formatter, "invalid decisive violation witness: {field}")
            }
            Self::InvariantLimitationsMismatch(field) => {
                write!(formatter, "outcome-invariant limitations disagree: {field}")
            }
            Self::SettlementWatermarkMismatch(field) => {
                write!(
                    formatter,
                    "tier settlement and Watermarks disagree: {field}"
                )
            }
            Self::InvalidEvidence(error) => write!(formatter, "invalid model evidence: {error}"),
            Self::InvalidRange(field) => write!(formatter, "invalid numeric range: {field}"),
            Self::RedactedAttributesWithoutPolicy => formatter.write_str(
                "non-empty redacted_attributes requires a configured redaction_policy_version",
            ),
            Self::Decode(error) => write!(formatter, "invalid protobuf bytes: {error}"),
        }
    }
}

impl Error for VerdictError {
    // Every variant is listed: no `_` arm, so adding or renaming one is a compile error.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidEvidence(error) => Some(error),
            Self::MissingScalar(_)
            | Self::MissingField(_)
            | Self::UnknownEnum { .. }
            | Self::UnspecifiedEnum(_)
            | Self::DuplicateValue(_)
            | Self::VerdictReasonMismatch
            | Self::AuthoritativeTierUnsettled
            | Self::ProvisionalTierMismatch
            | Self::AuthoritativeBarrierPending
            | Self::ClearedBarrierIncomplete
            | Self::UndeclaredBarrierHasTarget
            | Self::AssuranceSourceMismatch(_)
            | Self::AuthoritativeCoverageIncomplete(_)
            | Self::InvalidAnchorEvidence
            | Self::SourceTierGapMismatch
            | Self::MissingAnchorEvidence
            | Self::MissingSettlementBasis
            | Self::SettlementBasisMismatch
            | Self::AuthoritativeEvidenceIncomplete(_)
            | Self::SettlementEvidenceIncomplete(_)
            | Self::InvalidDecisiveWitness(_)
            | Self::InvariantLimitationsMismatch(_)
            | Self::SettlementWatermarkMismatch(_)
            | Self::InvalidRange(_)
            | Self::RedactedAttributesWithoutPolicy
            | Self::Decode(_) => None,
        }
    }
}

impl From<WireError> for VerdictError {
    fn from(error: WireError) -> Self {
        match error {
            WireError::MissingField(field) => Self::MissingField(field),
            WireError::DuplicateValue(field) => Self::DuplicateValue(field),
            WireError::InvalidEvidence(error) => Self::InvalidEvidence(error),
        }
    }
}

fn require_text(value: &str, field: &'static str) -> Result<(), VerdictError> {
    if value.trim().is_empty() {
        Err(VerdictError::MissingScalar(field))
    } else {
        Ok(())
    }
}

/// Validates retention/redaction claims on the Assurance Envelope (#206 / #243 / #244).
///
/// Absence of `raw_event_retention_days` or of either oneof is a defect. Configured
/// evidence-extract days must be inside `1..=365`; configured redaction versions must be
/// non-blank. `0` / `""` are never `NotDeployed` / `NoPolicyConfigured` encodings — those
/// use empty-message arms. There is no `evidence_extract ≤ raw_event` coupling on the
/// same envelope. Forecast-at-settle `redacted_attributes` may be empty under any policy
/// claim; a non-empty inventory under `RedactionPolicyNone` is rejected.
fn validate_retention(assurance: &raw_model::AssuranceEnvelope) -> Result<(), VerdictError> {
    let days = assurance
        .raw_event_retention_days
        .ok_or(VerdictError::MissingField(
            "assurance.raw_event_retention_days",
        ))?;
    let _ = RawEventRetentionDays::try_new(days)?;

    match assurance.evidence_extract_retention.as_ref() {
        None => {
            return Err(VerdictError::MissingField(
                "assurance.evidence_extract_retention",
            ))
        }
        Some(
            raw_model::assurance_envelope::EvidenceExtractRetention::EvidenceExtractRetentionDays(
                days,
            ),
        ) => {
            let _ = EvidenceExtractRetentionDays::try_new(*days)?;
        }
        Some(
            raw_model::assurance_envelope::EvidenceExtractRetention::EvidenceExtractNotDeployed(_),
        ) => {}
    }

    match assurance.redaction_policy.as_ref() {
        None => return Err(VerdictError::MissingField("assurance.redaction_policy")),
        Some(raw_model::assurance_envelope::RedactionPolicy::RedactionPolicyVersion(version))
            if version.trim().is_empty() =>
        {
            return Err(VerdictError::MissingScalar(
                "assurance.redaction_policy_version",
            ))
        }
        Some(
            raw_model::assurance_envelope::RedactionPolicy::RedactionPolicyVersion(_)
            | raw_model::assurance_envelope::RedactionPolicy::RedactionPolicyNone(_),
        ) => {}
    }

    validate_redacted_attributes(assurance)?;

    Ok(())
}

/// Validates the forecast-at-settle redacted-attribute inventory (#244).
///
/// Empty means only “no claims recorded.” Rejects blank `event_id` /
/// `attribute_path`, duplicate `(event_id, attribute_path)` pairs, and a
/// non-empty list under `redaction_policy` None.
fn validate_redacted_attributes(
    assurance: &raw_model::AssuranceEnvelope,
) -> Result<(), VerdictError> {
    if !assurance.redacted_attributes.is_empty()
        && matches!(
            assurance.redaction_policy.as_ref(),
            Some(raw_model::assurance_envelope::RedactionPolicy::RedactionPolicyNone(_))
        )
    {
        return Err(VerdictError::RedactedAttributesWithoutPolicy);
    }

    let mut unique = BTreeSet::new();
    for entry in &assurance.redacted_attributes {
        require_text(&entry.event_id, "assurance.redacted_attributes.event_id")?;
        require_text(
            &entry.attribute_path,
            "assurance.redacted_attributes.attribute_path",
        )?;
        if !unique.insert((entry.event_id.as_str(), entry.attribute_path.as_str())) {
            return Err(VerdictError::DuplicateValue(
                "assurance.redacted_attributes",
            ));
        }
    }
    Ok(())
}

fn validate_unique_texts(
    values: &[String],
    field: &'static str,
    required: bool,
) -> Result<(), VerdictError> {
    if required && values.is_empty() {
        return Err(VerdictError::MissingField(field));
    }
    let mut unique = BTreeSet::new();
    for value in values {
        require_text(value, field)?;
        if !unique.insert(value) {
            return Err(VerdictError::DuplicateValue(field));
        }
    }
    Ok(())
}

fn validate_source_set(
    sources: &[raw_model::SourceIdentity],
    field: &'static str,
    required: bool,
) -> Result<BTreeSet<SourceKey>, VerdictError> {
    if required && sources.is_empty() {
        return Err(VerdictError::MissingField(field));
    }
    let mut unique = BTreeSet::new();
    for source in sources {
        validate_source(source)?;
        if !unique.insert(owned_source_key(source)) {
            return Err(VerdictError::DuplicateValue(field));
        }
    }
    Ok(unique)
}

fn validate_filter_epochs(
    epochs: &[raw_model::FilterEpochBySource],
    barrier: raw_model::ActivationBarrierStatus,
) -> Result<BTreeSet<SourceKey>, VerdictError> {
    let mut sources = BTreeSet::new();
    for epoch in epochs {
        let source = epoch
            .source
            .as_ref()
            .ok_or(VerdictError::MissingField("assurance.filter_epoch.source"))?;
        validate_source(source)?;
        if epoch.filter_epoch.is_none() {
            match barrier {
                raw_model::ActivationBarrierStatus::Pending => {}
                raw_model::ActivationBarrierStatus::Cleared => {
                    return Err(VerdictError::ClearedBarrierIncomplete);
                }
                raw_model::ActivationBarrierStatus::NotDeclared
                | raw_model::ActivationBarrierStatus::Unspecified => {
                    return Err(VerdictError::MissingScalar(
                        "assurance.filter_epoch.filter_epoch",
                    ));
                }
            }
        }
        if !sources.insert(owned_source_key(source)) {
            return Err(VerdictError::DuplicateValue("assurance.filter_epochs"));
        }
    }
    Ok(sources)
}

fn validate_declared_barrier(
    assurance: &raw_model::AssuranceEnvelope,
    barrier: raw_model::ActivationBarrierStatus,
    required: &BTreeSet<SourceKey>,
    filter_epochs: &BTreeSet<SourceKey>,
) -> Result<(), VerdictError> {
    match barrier {
        raw_model::ActivationBarrierStatus::Unspecified => Err(VerdictError::UnspecifiedEnum(
            "assurance.activation_barrier",
        )),
        raw_model::ActivationBarrierStatus::NotDeclared => {
            if assurance.target_filter_epoch.is_some() {
                Err(VerdictError::UndeclaredBarrierHasTarget)
            } else {
                Ok(())
            }
        }
        raw_model::ActivationBarrierStatus::Pending => Ok(()),
        raw_model::ActivationBarrierStatus::Cleared => {
            let Some(target) = assurance.target_filter_epoch else {
                return Err(VerdictError::ClearedBarrierIncomplete);
            };
            if filter_epochs != required {
                return Err(VerdictError::ClearedBarrierIncomplete);
            }
            for epoch in &assurance.filter_epochs {
                let Some(acknowledged) = epoch.filter_epoch else {
                    return Err(VerdictError::ClearedBarrierIncomplete);
                };
                if acknowledged < target {
                    return Err(VerdictError::ClearedBarrierIncomplete);
                }
            }
            Ok(())
        }
    }
}

fn validate_gaps(gaps: &[raw_model::SourceGap]) -> Result<BTreeSet<SourceKey>, VerdictError> {
    let mut sources = BTreeSet::new();
    let mut unique = BTreeSet::new();
    for gap in gaps {
        let source = gap
            .source
            .as_ref()
            .ok_or(VerdictError::MissingField("assurance.gap.source"))?;
        validate_source(source)?;
        sources.insert(owned_source_key(source));
        require_text(&gap.effect, "assurance.gap.effect")?;
        if gap.first_missing > gap.last_missing {
            return Err(VerdictError::InvalidRange("assurance.gap"));
        }
        if !unique.insert((
            source_key(source),
            gap.sequence_epoch,
            gap.first_missing,
            gap.last_missing,
        )) {
            return Err(VerdictError::DuplicateValue("assurance.gaps"));
        }
    }
    Ok(sources)
}

fn validate_source_health(
    records: &[raw_model::SourceHealth],
) -> Result<BTreeSet<SourceKey>, VerdictError> {
    let mut sources = BTreeSet::new();
    for record in records {
        let source = record
            .source
            .as_ref()
            .ok_or(VerdictError::MissingField("assurance.source_health.source"))?;
        validate_source(source)?;
        if !sources.insert(owned_source_key(source)) {
            return Err(VerdictError::DuplicateValue("assurance.source_health"));
        }
        if !record.healthy {
            require_text(&record.detail, "assurance.source_health.detail")?;
        }
    }
    Ok(sources)
}

fn validate_capability_records(
    records: &[raw_model::SourceCapabilityRecord],
) -> Result<BTreeSet<SourceKey>, VerdictError> {
    let mut sources = BTreeSet::new();
    for record in records {
        let source = record
            .source
            .as_ref()
            .ok_or(VerdictError::MissingField("assurance.capability.source"))?;
        validate_source(source)?;
        if !sources.insert(owned_source_key(source)) {
            return Err(VerdictError::DuplicateValue("assurance.capabilities"));
        }
        let mut capabilities = BTreeSet::new();
        for &value in &record.capabilities {
            match raw_model::SourceCapability::try_from(value) {
                Ok(raw_model::SourceCapability::Unspecified) => {
                    return Err(VerdictError::UnspecifiedEnum("assurance.source_capability"))
                }
                Ok(_) => {}
                Err(_) => {
                    return Err(VerdictError::UnknownEnum {
                        field: "assurance.source_capability",
                        value,
                    })
                }
            }
            if !capabilities.insert(value) {
                return Err(VerdictError::DuplicateValue(
                    "assurance.source_capabilities",
                ));
            }
        }
    }
    Ok(sources)
}

/// The Source Tier classification and the per-Verdict anchor evidence, keyed by Source.
///
/// `tiers` is what ADR-0022 classifies a Source as; `anchored_event_ids` is what that
/// Source actually anchored *for this Verdict*, which is evidence rather than a
/// restatement of the tier. The two are deliberately asymmetric -- read the comment in
/// `validate_tier_records` before treating an empty anchor list on an Anchor-tier Source
/// as a defect.
struct TierEvidence {
    sources: BTreeSet<SourceKey>,
    tiers: BTreeMap<SourceKey, raw_model::SourceTier>,
    anchored_event_ids: BTreeMap<SourceKey, BTreeSet<String>>,
}

impl TierEvidence {
    fn has_valid_anchor(&self) -> bool {
        !self.valid_anchor_sources().is_empty()
    }

    fn valid_anchor_sources(&self) -> BTreeSet<SourceKey> {
        self.tiers
            .iter()
            .filter(|(source, tier)| {
                **tier == raw_model::SourceTier::Anchor
                    && self
                        .anchored_event_ids
                        .get(*source)
                        .is_some_and(|events| !events.is_empty())
            })
            .map(|(source, _)| source.clone())
            .collect()
    }

    fn contributing_sources(&self) -> BTreeSet<SourceKey> {
        self.tiers
            .iter()
            .filter(|(_, tier)| **tier == raw_model::SourceTier::Contributing)
            .map(|(source, _)| source.clone())
            .collect()
    }
}

fn validate_tier_records(
    records: &[raw_model::SourceTierRecord],
) -> Result<TierEvidence, VerdictError> {
    let mut sources = BTreeSet::new();
    let mut tiers = BTreeMap::new();
    let mut anchored_event_ids = BTreeMap::new();
    for record in records {
        let source = record
            .source
            .as_ref()
            .ok_or(VerdictError::MissingField("assurance.source_tier.source"))?;
        validate_source(source)?;
        if !sources.insert(owned_source_key(source)) {
            return Err(VerdictError::DuplicateValue(
                "assurance.source_tiers_and_anchors",
            ));
        }
        let tier = match raw_model::SourceTier::try_from(record.tier) {
            Ok(raw_model::SourceTier::Unspecified) => {
                return Err(VerdictError::UnspecifiedEnum("assurance.source_tier"))
            }
            Ok(tier) => tier,
            Err(_) => {
                return Err(VerdictError::UnknownEnum {
                    field: "assurance.source_tier",
                    value: record.tier,
                })
            }
        };
        validate_unique_texts(
            &record.anchored_event_ids,
            "assurance.anchored_event_ids",
            false,
        )?;
        // Listing anchored Events for a source that by classification cannot anchor is a
        // category error, so it is rejected here. The apparent mirror -- an `Anchor`-tier
        // record with empty `anchored_event_ids` -- is deliberately *not* rejected, and is
        // deliberately not folded into the source-tier gap trigger either. Per ADR-0022,
        // `anchored_event_ids` is per-Verdict evidence rather than a restatement of the tier:
        // an empty list means this anchor source anchored no Event for this Verdict, and
        // silence at a watermark-complete, healthy anchor source is itself evidence. The
        // assurance downgrade is owed collectively (no required source carries valid anchor
        // evidence), never per source, so a Verdict with at least one genuine anchor is not
        // downgraded because a co-required anchor source was quiet. Such a source is still
        // named in `unanchored_sources` and never in `contributing_sources`. Before "fixing"
        // this asymmetry, read the Costs section of ADR-0022 and issue #47.
        if tier == raw_model::SourceTier::Contributing && !record.anchored_event_ids.is_empty() {
            return Err(VerdictError::InvalidAnchorEvidence);
        }
        let key = owned_source_key(source);
        tiers.insert(key.clone(), tier);
        anchored_event_ids.insert(key, record.anchored_event_ids.iter().cloned().collect());
    }
    Ok(TierEvidence {
        sources,
        tiers,
        anchored_event_ids,
    })
}

fn validate_causal_edges(edges: &[raw_model::UnresolvedCausalEdge]) -> Result<(), VerdictError> {
    let mut unique = BTreeSet::new();
    for edge in edges {
        require_text(&edge.event_id, "assurance.unresolved_edge.event_id")?;
        require_text(
            &edge.missing_predecessor_id,
            "assurance.unresolved_edge.missing_predecessor_id",
        )?;
        if !unique.insert((edge.event_id.as_str(), edge.missing_predecessor_id.as_str())) {
            return Err(VerdictError::DuplicateValue(
                "assurance.unresolved_causal_edges",
            ));
        }
    }
    Ok(())
}

fn validate_extraction_facts(facts: &[raw_model::ExtractionFact]) -> Result<(), VerdictError> {
    let mut unique = BTreeSet::new();
    for fact in facts {
        require_text(&fact.fact_id, "assurance.extraction_fact.fact_id")?;
        if !unique.insert(fact.fact_id.as_str()) {
            return Err(VerdictError::DuplicateValue("assurance.extraction_facts"));
        }
        if !fact.available {
            require_text(&fact.detail, "assurance.extraction_fact.detail")?;
        }
    }
    Ok(())
}

fn validate_uncertainty(values: &[raw_model::Uncertainty]) -> Result<(), VerdictError> {
    let mut reasons = BTreeSet::new();
    for uncertainty in values {
        match raw_model::InconclusiveReason::try_from(uncertainty.reason) {
            Ok(raw_model::InconclusiveReason::Unspecified) => {
                return Err(VerdictError::UnspecifiedEnum(
                    "assurance.uncertainty.reason",
                ))
            }
            Ok(_) => {}
            Err(_) => {
                return Err(VerdictError::UnknownEnum {
                    field: "assurance.uncertainty.reason",
                    value: uncertainty.reason,
                })
            }
        }
        if !reasons.insert(uncertainty.reason) {
            return Err(VerdictError::DuplicateValue("assurance.uncertainty"));
        }
        require_text(&uncertainty.detail, "assurance.uncertainty.detail")?;
    }
    Ok(())
}

type GapKey = (SourceKey, u64, u64, u64, String);
type EdgeKey = (String, String);
type WatermarkCoordinate = (u64, u64);

fn gap_keys(gaps: &[raw_model::SourceGap]) -> BTreeSet<GapKey> {
    gaps.iter()
        .filter_map(|gap| {
            gap.source.as_ref().map(|source| {
                (
                    owned_source_key(source),
                    gap.sequence_epoch,
                    gap.first_missing,
                    gap.last_missing,
                    gap.effect.clone(),
                )
            })
        })
        .collect()
}

fn edge_keys(edges: &[raw_model::UnresolvedCausalEdge]) -> BTreeSet<EdgeKey> {
    edges
        .iter()
        .map(|edge| (edge.event_id.clone(), edge.missing_predecessor_id.clone()))
        .collect()
}

fn watermark_coordinates(
    watermarks: &[raw_model::Watermark],
) -> BTreeMap<SourceKey, WatermarkCoordinate> {
    watermarks
        .iter()
        .filter_map(|watermark| {
            watermark.source.as_ref().map(|source| {
                (
                    owned_source_key(source),
                    (watermark.sequence_epoch, watermark.complete_through),
                )
            })
        })
        .collect()
}

fn health_by_source(records: &[raw_model::SourceHealth]) -> BTreeMap<SourceKey, bool> {
    records
        .iter()
        .filter_map(|record| {
            record
                .source
                .as_ref()
                .map(|source| (owned_source_key(source), record.healthy))
        })
        .collect()
}

fn extraction_facts_by_id(facts: &[raw_model::ExtractionFact]) -> BTreeMap<&str, bool> {
    facts
        .iter()
        .map(|fact| (fact.fact_id.as_str(), fact.available))
        .collect()
}

fn uncertainty_reasons(values: &[raw_model::Uncertainty]) -> BTreeSet<i32> {
    values.iter().map(|value| value.reason).collect()
}

fn silent_anchor_keys(basis: Option<&raw_model::SettlementBasis>) -> BTreeSet<SourceKey> {
    let Some(raw_model::settlement_basis::Basis::WatermarkAnchoredViolation(watermarked)) =
        basis.and_then(|basis| basis.basis.as_ref())
    else {
        return BTreeSet::new();
    };
    watermarked
        .silent_anchor_sources
        .iter()
        .map(owned_source_key)
        .collect()
}

fn validate_uncertainty_reason_set(
    values: &[i32],
    field: &'static str,
) -> Result<BTreeSet<i32>, VerdictError> {
    let mut reasons = BTreeSet::new();
    for &value in values {
        match raw_model::InconclusiveReason::try_from(value) {
            Ok(raw_model::InconclusiveReason::Unspecified) => {
                return Err(VerdictError::UnspecifiedEnum(field))
            }
            Ok(_) => {}
            Err(_) => return Err(VerdictError::UnknownEnum { field, value }),
        }
        if !reasons.insert(value) {
            return Err(VerdictError::DuplicateValue(field));
        }
    }
    Ok(reasons)
}

/// Everything the Assurance Envelope's outcome-basis rules need, collected exactly once.
///
/// The intermediates are owned rather than borrowed so that the value carries its own
/// derived evidence instead of pointing at locals in whichever function happened to
/// build them. `hot_path` and `cold_path` are deliberately absent: they are consumed by
/// the settlement-watermark checks over the *key sets* and are never read again once the
/// coordinate maps exist.
///
/// This type must not gain `Clone`, `Default`, or a `Drop` impl. See [`Self::new`] for
/// why each one would break something: the first two would let a caller sidestep the
/// constructor that establishes every field, and the third would extend the borrow of
/// `assurance` past the validator's last use, so `Verdict::try_from` could no longer
/// take `&mut value.assurance` to canonicalize.
struct AssuranceValidator<'a> {
    assurance: &'a raw_model::AssuranceEnvelope,
    outcome: raw_model::VerdictOutcome,
    authority: raw_model::VerdictAuthority,
    required: BTreeSet<SourceKey>,
    observed: BTreeSet<SourceKey>,
    tier_evidence: TierEvidence,
    watermarks: BTreeMap<SourceKey, WatermarkCoordinate>,
    hot_watermarks: BTreeMap<SourceKey, WatermarkCoordinate>,
    cold_watermarks: BTreeMap<SourceKey, WatermarkCoordinate>,
}

impl<'a> AssuranceValidator<'a> {
    /// Validate an Assurance Envelope against the authority and outcome it is offered for.
    ///
    /// The single entry point for envelope validation: every rule that reads the envelope
    /// runs from here, in the order it ran before this type existed. That includes the
    /// three header claims -- the hot and cold Tier Settlement enums, the Late Data Policy
    /// and the authority/tier-settlement agreement ADR-0021 requires -- which used to sit
    /// in `Verdict::try_from`. They are claims about the Assurance Envelope, so leaving
    /// them outside would have meant this type validated all of the envelope except the
    /// three fields a test is most likely to get wrong.
    ///
    /// Deliberately free-function-shaped: envelope in, `Result<(), _>` out, no value handed
    /// back. `Verdict::try_from` therefore has exactly the borrow structure it had when
    /// this was a free function, and can still take `&mut value.assurance` on the next line
    /// to canonicalize.
    fn check(
        assurance: &'a raw_model::AssuranceEnvelope,
        outcome: raw_model::VerdictOutcome,
        authority: raw_model::VerdictAuthority,
    ) -> Result<(), VerdictError> {
        let hot = raw_model::TierSettlement::try_from(assurance.hot_path).map_err(|_| {
            VerdictError::UnknownEnum {
                field: "assurance.hot_path",
                value: assurance.hot_path,
            }
        })?;
        let cold = raw_model::TierSettlement::try_from(assurance.cold_path).map_err(|_| {
            VerdictError::UnknownEnum {
                field: "assurance.cold_path",
                value: assurance.cold_path,
            }
        })?;
        if hot == raw_model::TierSettlement::Unspecified
            || cold == raw_model::TierSettlement::Unspecified
        {
            return Err(VerdictError::UnspecifiedEnum("assurance.tier_settlement"));
        }
        let late_policy =
            raw_model::LateDataPolicy::try_from(assurance.late_data_policy).map_err(|_| {
                VerdictError::UnknownEnum {
                    field: "assurance.late_data_policy",
                    value: assurance.late_data_policy,
                }
            })?;
        if late_policy == raw_model::LateDataPolicy::Unspecified {
            return Err(VerdictError::UnspecifiedEnum("assurance.late_data_policy"));
        }
        let barrier = raw_model::ActivationBarrierStatus::try_from(assurance.activation_barrier)
            .map_err(|_| VerdictError::UnknownEnum {
                field: "assurance.activation_barrier",
                value: assurance.activation_barrier,
            })?;
        if barrier == raw_model::ActivationBarrierStatus::Unspecified {
            return Err(VerdictError::UnspecifiedEnum(
                "assurance.activation_barrier",
            ));
        }
        // Barrier first: a hand-built Authoritative + PENDING + cold == PENDING is a
        // rollout lie, not a tier lie. Core never emits that pair once authority is
        // Provisional if cold == PENDING || barrier.is_pending().
        match authority {
            raw_model::VerdictAuthority::Authoritative
                if barrier == raw_model::ActivationBarrierStatus::Pending =>
            {
                return Err(VerdictError::AuthoritativeBarrierPending)
            }
            raw_model::VerdictAuthority::Authoritative
                if hot != raw_model::TierSettlement::Settled
                    || !matches!(
                        cold,
                        raw_model::TierSettlement::Settled | raw_model::TierSettlement::NotRequired
                    ) =>
            {
                return Err(VerdictError::AuthoritativeTierUnsettled)
            }
            raw_model::VerdictAuthority::Provisional
                if hot != raw_model::TierSettlement::Settled
                    || (cold != raw_model::TierSettlement::Pending
                        && barrier != raw_model::ActivationBarrierStatus::Pending) =>
            {
                return Err(VerdictError::ProvisionalTierMismatch)
            }
            _ => {}
        }
        Self::new(assurance, outcome, authority, hot, cold, barrier)?.validate()
    }

    /// Collect every intermediate the outcome-basis rules need, validating as it goes.
    ///
    /// Only [`AssuranceValidator::check`] may call this. `check` establishes the header
    /// preconditions -- it parses `hot_path`/`cold_path` and rejects `Unspecified` before
    /// this function ever sees them -- so calling `new` directly with hand-supplied
    /// settlements would reach the authority-tier match with values `Verdict::try_from`
    /// rejects earlier, and the two entry points would disagree about the same envelope.
    ///
    /// The only constructor, and it is total: it either fails or hands back a validator
    /// whose evidence collections have all been checked, individually and against the
    /// required source set. There is no half-built state to defend against, because there
    /// is no other way to obtain a `Self` -- which is what lets the rules below read the
    /// fields without re-establishing that they are coherent.
    ///
    /// For that reason the type must not gain `Default` or `Clone`: either would hand out
    /// a value whose fields did not come from here, and every method's precondition would
    /// go back to being a comment. It must not implement `Drop` either -- there is nothing
    /// to release, and a destructor would make the envelope borrow outlive the last read
    /// of it for any caller that binds the validator to a name instead of calling
    /// [`AssuranceValidator::check`].
    ///
    /// The subset, authoritative-coverage and settlement-watermark checks stay here rather
    /// than becoming methods, because they run over the watermark *key sets* -- the sets
    /// of Sources that carry a Watermark at all. The coordinate maps that shadow those
    /// names at the end of this function are a different thing, positions rather than
    /// membership, and are what the basis rules need. The two are not interchangeable.
    fn new(
        assurance: &'a raw_model::AssuranceEnvelope,
        outcome: raw_model::VerdictOutcome,
        authority: raw_model::VerdictAuthority,
        hot: raw_model::TierSettlement,
        cold: raw_model::TierSettlement,
        barrier: raw_model::ActivationBarrierStatus,
    ) -> Result<Self, VerdictError> {
        require_text(&assurance.specification_id, "assurance.specification_id")?;
        require_text(
            &assurance.specification_version,
            "assurance.specification_version",
        )?;
        validate_retention(assurance)?;
        validate_unique_texts(
            &assurance.schema_versions,
            "assurance.schema_versions",
            true,
        )?;
        let required = validate_source_set(
            &assurance.required_sources,
            "assurance.required_sources",
            true,
        )?;
        let observed = validate_source_set(
            &assurance.observed_sources,
            "assurance.observed_sources",
            false,
        )?;
        let filter_epochs = validate_filter_epochs(&assurance.filter_epochs, barrier)?;
        validate_declared_barrier(assurance, barrier, &required, &filter_epochs)?;
        let watermarks = validate_watermarks(&assurance.watermarks, "assurance.watermarks")?;
        let gaps = validate_gaps(&assurance.gaps)?;
        let source_health = validate_source_health(&assurance.source_health)?;
        let capabilities = validate_capability_records(&assurance.capabilities)?;
        let tier_evidence = validate_tier_records(&assurance.source_tiers_and_anchors)?;
        validate_causal_edges(&assurance.unresolved_causal_edges)?;
        validate_extraction_facts(&assurance.extraction_facts)?;
        validate_uncertainty(&assurance.uncertainty)?;
        let hot_watermarks = validate_watermarks(
            &assurance.hot_settlement_watermarks,
            "assurance.hot_settlement_watermarks",
        )?;
        let cold_watermarks = validate_watermarks(
            &assurance.cold_settlement_watermarks,
            "assurance.cold_settlement_watermarks",
        )?;

        for (field, sources) in [
            ("assurance.observed_sources", &observed),
            ("assurance.filter_epochs", &filter_epochs),
            ("assurance.watermarks", &watermarks),
            ("assurance.gaps", &gaps),
            ("assurance.source_health", &source_health),
            ("assurance.capabilities", &capabilities),
            ("assurance.source_tiers_and_anchors", &tier_evidence.sources),
            ("assurance.hot_settlement_watermarks", &hot_watermarks),
            ("assurance.cold_settlement_watermarks", &cold_watermarks),
        ] {
            Self::require_subset(sources, &required, field)?;
        }

        let authoritative = authority == raw_model::VerdictAuthority::Authoritative;
        if authoritative {
            for (field, sources) in [
                ("assurance.observed_sources", &observed),
                ("assurance.filter_epochs", &filter_epochs),
                ("assurance.watermarks", &watermarks),
                ("assurance.source_health", &source_health),
                ("assurance.capabilities", &capabilities),
                ("assurance.source_tiers_and_anchors", &tier_evidence.sources),
            ] {
                Self::require_authoritative_coverage(sources, &required, field)?;
            }
        }
        Self::validate_source_tier_gap(
            assurance.source_tier_gap.as_ref(),
            &required,
            &tier_evidence,
            &silent_anchor_keys(assurance.settlement_basis.as_ref()),
        )?;
        Self::validate_settlement_watermarks(
            hot,
            &hot_watermarks,
            &required,
            "assurance.hot_settlement_watermarks",
            authoritative,
        )?;
        Self::validate_settlement_watermarks(
            cold,
            &cold_watermarks,
            &required,
            "assurance.cold_settlement_watermarks",
            authoritative,
        )?;
        let watermarks = watermark_coordinates(&assurance.watermarks);
        let hot_watermarks = watermark_coordinates(&assurance.hot_settlement_watermarks);
        let cold_watermarks = watermark_coordinates(&assurance.cold_settlement_watermarks);
        Ok(Self {
            assurance,
            outcome,
            authority,
            required,
            observed,
            tier_evidence,
            watermarks,
            hot_watermarks,
            cold_watermarks,
        })
    }

    fn require_subset(
        actual: &BTreeSet<SourceKey>,
        required: &BTreeSet<SourceKey>,
        field: &'static str,
    ) -> Result<(), VerdictError> {
        if actual.is_subset(required) {
            Ok(())
        } else {
            Err(VerdictError::AssuranceSourceMismatch(field))
        }
    }

    fn require_authoritative_coverage(
        actual: &BTreeSet<SourceKey>,
        required: &BTreeSet<SourceKey>,
        field: &'static str,
    ) -> Result<(), VerdictError> {
        if actual == required {
            Ok(())
        } else {
            Err(VerdictError::AuthoritativeCoverageIncomplete(field))
        }
    }

    fn validate_source_tier_gap(
        gap: Option<&raw_model::SourceTierGap>,
        required: &BTreeSet<SourceKey>,
        tier_evidence: &TierEvidence,
        watermark_silent_anchors: &BTreeSet<SourceKey>,
    ) -> Result<(), VerdictError> {
        let mut valid_anchor_sources = tier_evidence.valid_anchor_sources();
        valid_anchor_sources.extend(watermark_silent_anchors.iter().cloned());
        let contributing_sources = tier_evidence.contributing_sources();
        let unanchored_sources = required
            .difference(&valid_anchor_sources)
            .cloned()
            .collect::<BTreeSet<_>>();
        let gap_required = tier_evidence.sources != *required || valid_anchor_sources.is_empty();

        if let Some(gap) = gap {
            require_text(&gap.detail, "assurance.source_tier_gap.detail")?;
            let claimed_contributing_sources = validate_source_set(
                &gap.contributing_sources,
                "assurance.source_tier_gap.contributing_sources",
                false,
            )?;
            Self::require_subset(
                &claimed_contributing_sources,
                required,
                "assurance.source_tier_gap.contributing_sources",
            )?;
            let claimed_unanchored_sources = validate_source_set(
                &gap.unanchored_sources,
                "assurance.source_tier_gap.unanchored_sources",
                true,
            )?;
            Self::require_subset(
                &claimed_unanchored_sources,
                required,
                "assurance.source_tier_gap.unanchored_sources",
            )?;
            if !gap_required
                || claimed_contributing_sources != contributing_sources
                || claimed_unanchored_sources != unanchored_sources
            {
                return Err(VerdictError::SourceTierGapMismatch);
            }
        } else if gap_required {
            return Err(VerdictError::SourceTierGapMismatch);
        }
        Ok(())
    }

    fn validate_settlement_watermarks(
        settlement: raw_model::TierSettlement,
        watermarks: &BTreeSet<SourceKey>,
        required: &BTreeSet<SourceKey>,
        field: &'static str,
        authoritative: bool,
    ) -> Result<(), VerdictError> {
        match settlement {
            raw_model::TierSettlement::Settled if watermarks.is_empty() => {
                Err(VerdictError::SettlementWatermarkMismatch(field))
            }
            raw_model::TierSettlement::Settled if authoritative => {
                Self::require_authoritative_coverage(watermarks, required, field)
            }
            raw_model::TierSettlement::Pending | raw_model::TierSettlement::NotRequired
                if !watermarks.is_empty() =>
            {
                Err(VerdictError::SettlementWatermarkMismatch(field))
            }
            raw_model::TierSettlement::Unspecified => {
                Err(VerdictError::UnspecifiedEnum("assurance.tier_settlement"))
            }
            raw_model::TierSettlement::Settled
            | raw_model::TierSettlement::Pending
            | raw_model::TierSettlement::NotRequired => Ok(()),
        }
    }

    fn validate_settlement_reached(
        &self,
        settlement: &BTreeMap<SourceKey, WatermarkCoordinate>,
        field: &'static str,
    ) -> Result<(), VerdictError> {
        for (source, settled_through) in settlement {
            match self.watermarks.get(source) {
                Some(observed_through) if observed_through >= settled_through => {}
                Some(_) | None => return Err(VerdictError::SettlementEvidenceIncomplete(field)),
            }
        }
        Ok(())
    }

    fn coordinate_is_settled(
        coordinate: WatermarkCoordinate,
        source: &SourceKey,
        watermarks: &BTreeMap<SourceKey, WatermarkCoordinate>,
    ) -> bool {
        watermarks
            .get(source)
            .is_some_and(|complete_through| complete_through >= &coordinate)
    }

    /// Dispatch on the claim the Verdict makes and check the basis it names.
    ///
    /// Consumes the validator: the evidence was collected for this one decision, and
    /// nothing downstream has any use for it afterwards.
    fn validate(self) -> Result<(), VerdictError> {
        let basis = self
            .assurance
            .settlement_basis
            .as_ref()
            .and_then(|basis| basis.basis.as_ref());
        match (self.authority, self.outcome, basis) {
            (
                raw_model::VerdictAuthority::Authoritative,
                raw_model::VerdictOutcome::Satisfy,
                Some(raw_model::settlement_basis::Basis::CompleteEvidence(_)),
            ) => self.validate_complete_evidence_basis(),
            (
                raw_model::VerdictAuthority::Authoritative,
                raw_model::VerdictOutcome::Violate,
                Some(raw_model::settlement_basis::Basis::DecisiveViolation(basis)),
            ) => self.validate_decisive_violation_basis(basis),
            (
                raw_model::VerdictAuthority::Authoritative,
                raw_model::VerdictOutcome::Violate,
                Some(raw_model::settlement_basis::Basis::WatermarkAnchoredViolation(basis)),
            ) => self.validate_watermark_anchored_violation_basis(basis),
            (
                raw_model::VerdictAuthority::Authoritative,
                raw_model::VerdictOutcome::Satisfy | raw_model::VerdictOutcome::Violate,
                None,
            ) => Err(VerdictError::MissingSettlementBasis),
            (raw_model::VerdictAuthority::Provisional, _, None)
            | (
                raw_model::VerdictAuthority::Authoritative,
                raw_model::VerdictOutcome::Inconclusive,
                None,
            ) => Ok(()),
            (_, _, Some(_)) => Err(VerdictError::SettlementBasisMismatch),
            (raw_model::VerdictAuthority::Unspecified, _, _)
            | (_, raw_model::VerdictOutcome::Unspecified, _) => {
                Err(VerdictError::SettlementBasisMismatch)
            }
        }
    }

    fn validate_complete_evidence_basis(&self) -> Result<(), VerdictError> {
        let assurance = self.assurance;
        if assurance.source_health.iter().any(|record| !record.healthy) {
            return Err(VerdictError::AuthoritativeEvidenceIncomplete(
                "assurance.source_health",
            ));
        }
        for (is_open, field) in [
            (!assurance.gaps.is_empty(), "assurance.gaps"),
            (
                !assurance.unresolved_causal_edges.is_empty(),
                "assurance.unresolved_causal_edges",
            ),
            (
                assurance
                    .extraction_facts
                    .iter()
                    .any(|fact| !fact.available),
                "assurance.extraction_facts",
            ),
            (!assurance.uncertainty.is_empty(), "assurance.uncertainty"),
        ] {
            if is_open {
                return Err(VerdictError::AuthoritativeEvidenceIncomplete(field));
            }
        }
        if assurance.source_tier_gap.is_some() || !self.tier_evidence.has_valid_anchor() {
            return Err(VerdictError::MissingAnchorEvidence);
        }
        self.validate_settlement_reached(
            &self.hot_watermarks,
            "assurance.hot_settlement_watermarks",
        )?;
        self.validate_settlement_reached(
            &self.cold_watermarks,
            "assurance.cold_settlement_watermarks",
        )
    }

    fn validate_decisive_violation_basis(
        &self,
        basis: &raw_model::DecisiveViolationBasis,
    ) -> Result<(), VerdictError> {
        self.validate_decisive_basis_header(basis)?;
        let health = health_by_source(&self.assurance.source_health);
        let unresolved_edges = edge_keys(&self.assurance.unresolved_causal_edges);
        let mut event_ids = BTreeSet::new();
        let mut anchored_witness = false;
        for event in &basis.witness_events {
            if !event_ids.insert(event.event_id.as_str()) {
                return Err(VerdictError::DuplicateValue(
                    "assurance.settlement_basis.witness_events",
                ));
            }
            anchored_witness |= self.validate_decisive_event(event, &health, &unresolved_edges)?;
        }
        if !anchored_witness || self.assurance.source_tier_gap.is_some() {
            return Err(VerdictError::MissingAnchorEvidence);
        }
        Ok(())
    }

    fn validate_watermark_anchored_violation_basis(
        &self,
        basis: &raw_model::WatermarkAnchoredViolationBasis,
    ) -> Result<(), VerdictError> {
        require_text(
            &basis.invariance_proof_rule_id,
            "assurance.settlement_basis.invariance_proof_rule_id",
        )?;
        require_text(
            &basis.invariance_proof_rule_version,
            "assurance.settlement_basis.invariance_proof_rule_version",
        )?;
        let silent = validate_source_set(
            &basis.silent_anchor_sources,
            "assurance.settlement_basis.silent_anchor_sources",
            true,
        )?;
        Self::require_subset(
            &silent,
            &self.required,
            "assurance.settlement_basis.silent_anchor_sources",
        )?;
        let silence_sources = validate_watermarks(
            &basis.silence_through_watermarks,
            "assurance.settlement_basis.silence_through_watermarks",
        )?;
        if silence_sources != silent {
            return Err(VerdictError::InvalidDecisiveWitness(
                "silence_through_watermarks",
            ));
        }
        let health = health_by_source(&self.assurance.source_health);
        for source in &silent {
            if !self.observed.contains(source) {
                return Err(VerdictError::InvalidDecisiveWitness(
                    "silent_anchor_sources.observed",
                ));
            }
            if self.tier_evidence.tiers.get(source) != Some(&raw_model::SourceTier::Anchor) {
                return Err(VerdictError::InvalidDecisiveWitness(
                    "silent_anchor_sources.tier",
                ));
            }
            if health.get(source) != Some(&true) {
                return Err(VerdictError::InvalidDecisiveWitness(
                    "silent_anchor_sources.health",
                ));
            }
            if self
                .tier_evidence
                .anchored_event_ids
                .get(source)
                .is_some_and(|events| !events.is_empty())
            {
                return Err(VerdictError::InvalidDecisiveWitness(
                    "silent_anchor_sources.anchored_event_ids",
                ));
            }
        }
        let limitations =
            basis
                .outcome_invariant_limitations
                .as_ref()
                .ok_or(VerdictError::MissingField(
                    "assurance.settlement_basis.outcome_invariant_limitations",
                ))?;
        self.validate_invariant_limitations(limitations)?;
        self.validate_settlement_reached(
            &self.hot_watermarks,
            "assurance.hot_settlement_watermarks",
        )?;
        self.validate_settlement_reached(
            &self.cold_watermarks,
            "assurance.cold_settlement_watermarks",
        )?;
        self.validate_settlement_reached(
            &watermark_coordinates(&basis.silence_through_watermarks),
            "assurance.settlement_basis.silence_through_watermarks",
        )?;
        if self.assurance.source_tier_gap.is_some() {
            return Err(VerdictError::MissingAnchorEvidence);
        }
        Ok(())
    }

    fn validate_decisive_basis_header(
        &self,
        basis: &raw_model::DecisiveViolationBasis,
    ) -> Result<(), VerdictError> {
        require_text(
            &basis.invariance_proof_rule_id,
            "assurance.settlement_basis.invariance_proof_rule_id",
        )?;
        require_text(
            &basis.invariance_proof_rule_version,
            "assurance.settlement_basis.invariance_proof_rule_version",
        )?;
        if basis.witness_events.is_empty() {
            return Err(VerdictError::InvalidDecisiveWitness("witness_events"));
        }
        validate_unique_texts(
            &basis.required_extraction_fact_ids,
            "assurance.settlement_basis.required_extraction_fact_ids",
            false,
        )?;
        let facts = extraction_facts_by_id(&self.assurance.extraction_facts);
        if basis
            .required_extraction_fact_ids
            .iter()
            .any(|fact_id| facts.get(fact_id.as_str()) != Some(&true))
        {
            return Err(VerdictError::InvalidDecisiveWitness(
                "required_extraction_fact_ids",
            ));
        }
        let limitations =
            basis
                .outcome_invariant_limitations
                .as_ref()
                .ok_or(VerdictError::MissingField(
                    "assurance.settlement_basis.outcome_invariant_limitations",
                ))?;
        self.validate_invariant_limitations(limitations)?;
        self.validate_settlement_reached(
            &self.hot_watermarks,
            "assurance.hot_settlement_watermarks",
        )?;
        self.validate_settlement_reached(
            &self.cold_watermarks,
            "assurance.cold_settlement_watermarks",
        )
    }

    fn validate_decisive_event(
        &self,
        event: &raw_model::DecisiveEvent,
        health: &BTreeMap<SourceKey, bool>,
        unresolved_edges: &BTreeSet<EdgeKey>,
    ) -> Result<bool, VerdictError> {
        require_text(
            &event.event_id,
            "assurance.settlement_basis.witness_event.event_id",
        )?;
        let source = event.source.as_ref().ok_or(VerdictError::MissingField(
            "assurance.settlement_basis.witness_event.source",
        ))?;
        validate_source(source)?;
        let source = owned_source_key(source);
        if !self.required.contains(&source) || !self.observed.contains(&source) {
            return Err(VerdictError::InvalidDecisiveWitness("source"));
        }
        let coordinate = (
            event.sequence_epoch.ok_or(VerdictError::MissingScalar(
                "assurance.settlement_basis.witness_event.sequence_epoch",
            ))?,
            event.sequence_number.ok_or(VerdictError::MissingScalar(
                "assurance.settlement_basis.witness_event.sequence_number",
            ))?,
        );
        if !Self::coordinate_is_settled(coordinate, &source, &self.watermarks)
            || !Self::coordinate_is_settled(coordinate, &source, &self.hot_watermarks)
            || (!self.cold_watermarks.is_empty()
                && !Self::coordinate_is_settled(coordinate, &source, &self.cold_watermarks))
        {
            return Err(VerdictError::InvalidDecisiveWitness("settlement"));
        }
        if health.get(&source) != Some(&true) {
            return Err(VerdictError::InvalidDecisiveWitness("source_health"));
        }
        if self.assurance.gaps.iter().any(|gap| {
            gap.source.as_ref().is_some_and(|gap_source| {
                owned_source_key(gap_source) == source
                    && gap.sequence_epoch == coordinate.0
                    && gap.first_missing <= coordinate.1
                    && coordinate.1 <= gap.last_missing
            })
        }) {
            return Err(VerdictError::InvalidDecisiveWitness("sequence_gap"));
        }
        if unresolved_edges.iter().any(|(event_id, predecessor_id)| {
            event_id == &event.event_id || predecessor_id == &event.event_id
        }) {
            return Err(VerdictError::InvalidDecisiveWitness("causal_edge"));
        }
        Ok(
            self.tier_evidence.tiers.get(&source) == Some(&raw_model::SourceTier::Anchor)
                && self
                    .tier_evidence
                    .anchored_event_ids
                    .get(&source)
                    .is_some_and(|anchored| anchored.contains(&event.event_id)),
        )
    }

    fn validate_invariant_limitations(
        &self,
        limitations: &raw_model::OutcomeInvariantLimitations,
    ) -> Result<(), VerdictError> {
        let assurance = self.assurance;
        let gap_sources = validate_gaps(&limitations.gaps)?;
        Self::require_subset(
            &gap_sources,
            &self.required,
            "assurance.settlement_basis.invariant_gaps",
        )?;
        if gap_keys(&limitations.gaps) != gap_keys(&assurance.gaps) {
            return Err(VerdictError::InvariantLimitationsMismatch("gaps"));
        }

        let unhealthy_sources = validate_source_set(
            &limitations.unhealthy_sources,
            "assurance.settlement_basis.unhealthy_sources",
            false,
        )?;
        Self::require_subset(
            &unhealthy_sources,
            &self.required,
            "assurance.settlement_basis.unhealthy_sources",
        )?;
        let actual_unhealthy: BTreeSet<_> = assurance
            .source_health
            .iter()
            .filter(|record| !record.healthy)
            .filter_map(|record| record.source.as_ref().map(owned_source_key))
            .collect();
        if unhealthy_sources != actual_unhealthy {
            return Err(VerdictError::InvariantLimitationsMismatch(
                "unhealthy_sources",
            ));
        }

        validate_causal_edges(&limitations.unresolved_causal_edges)?;
        if edge_keys(&limitations.unresolved_causal_edges)
            != edge_keys(&assurance.unresolved_causal_edges)
        {
            return Err(VerdictError::InvariantLimitationsMismatch(
                "unresolved_causal_edges",
            ));
        }

        validate_unique_texts(
            &limitations.unavailable_extraction_fact_ids,
            "assurance.settlement_basis.unavailable_extraction_fact_ids",
            false,
        )?;
        let actual_unavailable: BTreeSet<_> = assurance
            .extraction_facts
            .iter()
            .filter(|fact| !fact.available)
            .map(|fact| fact.fact_id.as_str())
            .collect();
        let claimed_unavailable: BTreeSet<_> = limitations
            .unavailable_extraction_fact_ids
            .iter()
            .map(String::as_str)
            .collect();
        if claimed_unavailable != actual_unavailable {
            return Err(VerdictError::InvariantLimitationsMismatch(
                "unavailable_extraction_fact_ids",
            ));
        }

        let claimed_uncertainty = validate_uncertainty_reason_set(
            &limitations.uncertainty,
            "assurance.settlement_basis.uncertainty",
        )?;
        if claimed_uncertainty != uncertainty_reasons(&assurance.uncertainty) {
            return Err(VerdictError::InvariantLimitationsMismatch("uncertainty"));
        }
        Ok(())
    }
}

fn sort_gaps(gaps: &mut [raw_model::SourceGap]) {
    gaps.sort_by(|left, right| {
        compare_optional_sources(left.source.as_ref(), right.source.as_ref())
            .then_with(|| left.sequence_epoch.cmp(&right.sequence_epoch))
            .then_with(|| left.first_missing.cmp(&right.first_missing))
            .then_with(|| left.last_missing.cmp(&right.last_missing))
            .then_with(|| left.effect.cmp(&right.effect))
    });
}

fn sort_causal_edges(edges: &mut [raw_model::UnresolvedCausalEdge]) {
    edges.sort_by(|left, right| {
        (&left.event_id, &left.missing_predecessor_id)
            .cmp(&(&right.event_id, &right.missing_predecessor_id))
    });
}

fn canonicalize_invariant_limitations(limitations: &mut raw_model::OutcomeInvariantLimitations) {
    sort_gaps(&mut limitations.gaps);
    sort_sources(&mut limitations.unhealthy_sources);
    sort_causal_edges(&mut limitations.unresolved_causal_edges);
    limitations.unavailable_extraction_fact_ids.sort();
    limitations.uncertainty.sort_unstable();
}

fn canonicalize_settlement_basis(basis: &mut raw_model::SettlementBasis) {
    match basis.basis.as_mut() {
        Some(raw_model::settlement_basis::Basis::DecisiveViolation(decisive)) => {
            decisive.witness_events.sort_by(|left, right| {
                left.event_id
                    .cmp(&right.event_id)
                    .then_with(|| {
                        compare_optional_sources(left.source.as_ref(), right.source.as_ref())
                    })
                    .then_with(|| left.sequence_epoch.cmp(&right.sequence_epoch))
                    .then_with(|| left.sequence_number.cmp(&right.sequence_number))
            });
            decisive.required_extraction_fact_ids.sort();
            if let Some(limitations) = &mut decisive.outcome_invariant_limitations {
                canonicalize_invariant_limitations(limitations);
            }
        }
        Some(raw_model::settlement_basis::Basis::WatermarkAnchoredViolation(watermarked)) => {
            sort_sources(&mut watermarked.silent_anchor_sources);
            sort_watermarks(&mut watermarked.silence_through_watermarks);
            if let Some(limitations) = &mut watermarked.outcome_invariant_limitations {
                canonicalize_invariant_limitations(limitations);
            }
        }
        Some(raw_model::settlement_basis::Basis::CompleteEvidence(_)) | None => {}
    }
}

fn canonicalize_assurance(assurance: &mut raw_model::AssuranceEnvelope) {
    assurance.schema_versions.sort();
    assurance.filter_epochs.sort_by(|left, right| {
        compare_optional_sources(left.source.as_ref(), right.source.as_ref())
            .then_with(|| left.filter_epoch.cmp(&right.filter_epoch))
    });
    sort_watermarks(&mut assurance.watermarks);
    sort_sources(&mut assurance.required_sources);
    sort_sources(&mut assurance.observed_sources);
    sort_gaps(&mut assurance.gaps);
    assurance.source_health.sort_by(|left, right| {
        compare_optional_sources(left.source.as_ref(), right.source.as_ref())
            .then_with(|| left.healthy.cmp(&right.healthy))
            .then_with(|| left.detail.cmp(&right.detail))
    });
    for record in &mut assurance.capabilities {
        record.capabilities.sort_unstable();
    }
    assurance.capabilities.sort_by(|left, right| {
        compare_optional_sources(left.source.as_ref(), right.source.as_ref())
    });
    for record in &mut assurance.source_tiers_and_anchors {
        record.anchored_event_ids.sort();
    }
    assurance.source_tiers_and_anchors.sort_by(|left, right| {
        compare_optional_sources(left.source.as_ref(), right.source.as_ref())
    });
    sort_causal_edges(&mut assurance.unresolved_causal_edges);
    assurance
        .extraction_facts
        .sort_by(|left, right| left.fact_id.cmp(&right.fact_id));
    assurance.uncertainty.sort_by(|left, right| {
        left.reason
            .cmp(&right.reason)
            .then_with(|| left.detail.cmp(&right.detail))
    });
    sort_watermarks(&mut assurance.hot_settlement_watermarks);
    sort_watermarks(&mut assurance.cold_settlement_watermarks);
    if let Some(gap) = &mut assurance.source_tier_gap {
        sort_sources(&mut gap.contributing_sources);
        sort_sources(&mut gap.unanchored_sources);
    }
    if let Some(basis) = &mut assurance.settlement_basis {
        canonicalize_settlement_basis(basis);
    }
    assurance.redacted_attributes.sort_by(|left, right| {
        left.event_id
            .cmp(&right.event_id)
            .then_with(|| left.attribute_path.cmp(&right.attribute_path))
    });
}

/// Conclusion a Verdict reaches about one Specification and Binding.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum VerdictOutcome {
    /// The obligation holds.
    Satisfy,
    /// The obligation is falsified.
    Violate,
    /// The evidence does not decide the obligation.
    Inconclusive,
}

impl VerdictOutcome {
    /// `pub(crate)` only so `wire_helpers`' bijectivity tests can check that
    /// [`Self::to_raw`] names the variant this reads back. Still crate-private:
    /// ADR-0024's constraint is that ingress not be reachable from outside, and
    /// untrusted input still enters only through the validating boundaries.
    pub(crate) fn from_raw(value: raw_model::VerdictOutcome) -> Option<Self> {
        match value {
            raw_model::VerdictOutcome::Unspecified => None,
            raw_model::VerdictOutcome::Satisfy => Some(Self::Satisfy),
            raw_model::VerdictOutcome::Violate => Some(Self::Violate),
            raw_model::VerdictOutcome::Inconclusive => Some(Self::Inconclusive),
        }
    }
}

/// Named reason an Inconclusive Verdict could not settle.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum InconclusiveReason {
    /// The observed prefix is open: evidence that would decide it may still arrive.
    PrefixUncertainty,
    /// The observed evidence orders the relevant Events only partially.
    ObservationalUncertainty,
    /// A fact the conclusion needs could not be extracted.
    EpistemicUncertainty,
    /// A required anchor-tier source is complete, healthy, and silent, but the
    /// Specification needs a presence that silence cannot confirm.
    AnchorSilence,
}

impl InconclusiveReason {
    /// `pub(crate)` only so `wire_helpers`' bijectivity tests can check that
    /// [`Self::to_raw`] names the variant this reads back. Still crate-private:
    /// ADR-0024's constraint is that ingress not be reachable from outside, and
    /// untrusted input still enters only through the validating boundaries.
    pub(crate) fn from_raw(value: raw_model::InconclusiveReason) -> Option<Self> {
        match value {
            raw_model::InconclusiveReason::Unspecified => None,
            raw_model::InconclusiveReason::PrefixUncertainty => Some(Self::PrefixUncertainty),
            raw_model::InconclusiveReason::ObservationalUncertainty => {
                Some(Self::ObservationalUncertainty)
            }
            raw_model::InconclusiveReason::EpistemicUncertainty => Some(Self::EpistemicUncertainty),
            raw_model::InconclusiveReason::AnchorSilence => Some(Self::AnchorSilence),
        }
    }
}

/// How much authority a Verdict claims for its conclusion.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum VerdictAuthority {
    /// Settled by the hot path and awaiting cold-path confirmation.
    Provisional,
    /// Settled with a justifying evidence basis.
    Authoritative,
}

impl VerdictAuthority {
    /// `pub(crate)` only so `wire_helpers`' bijectivity tests can check that
    /// [`Self::to_raw`] names the variant this reads back. Still crate-private:
    /// ADR-0024's constraint is that ingress not be reachable from outside, and
    /// untrusted input still enters only through the validating boundaries.
    pub(crate) fn from_raw(value: raw_model::VerdictAuthority) -> Option<Self> {
        match value {
            raw_model::VerdictAuthority::Unspecified => None,
            raw_model::VerdictAuthority::Provisional => Some(Self::Provisional),
            raw_model::VerdictAuthority::Authoritative => Some(Self::Authoritative),
        }
    }
}

/// Settlement state of one evaluation tier.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum TierSettlement {
    /// The Specification needs no guard from this tier.
    NotRequired,
    /// This tier has not settled yet.
    Pending,
    /// This tier has settled.
    Settled,
}

impl TierSettlement {
    /// `pub(crate)` only so `wire_helpers`' bijectivity tests can check that
    /// [`Self::to_raw`] names the variant this reads back. Still crate-private:
    /// ADR-0024's constraint is that ingress not be reachable from outside, and
    /// untrusted input still enters only through the validating boundaries.
    pub(crate) fn from_raw(value: raw_model::TierSettlement) -> Option<Self> {
        match value {
            raw_model::TierSettlement::Unspecified => None,
            raw_model::TierSettlement::NotRequired => Some(Self::NotRequired),
            raw_model::TierSettlement::Pending => Some(Self::Pending),
            raw_model::TierSettlement::Settled => Some(Self::Settled),
        }
    }
}

/// Whether a Verdict was computed while a Specification was warming behind an
/// activation barrier.
///
/// Three-way: [`Self::NotDeclared`] is distinct from [`Self::Cleared`] so a consumer
/// cannot read "cleared" as evidence that a rollout completed when none was declared.
/// [`raw_model::ActivationBarrierStatus::Unspecified`] is not constructible.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum ActivationBarrierStatus {
    /// The Specification declared no activation barrier.
    NotDeclared,
    /// A declared barrier has not cleared.
    Pending,
    /// A declared barrier cleared.
    Cleared,
}

impl ActivationBarrierStatus {
    /// `pub(crate)` only so `wire_helpers`' bijectivity tests can check that
    /// [`Self::to_raw`] names the variant this reads back. Still crate-private:
    /// ADR-0024's constraint is that ingress not be reachable from outside, and
    /// untrusted input still enters only through the validating boundaries.
    pub(crate) fn from_raw(value: raw_model::ActivationBarrierStatus) -> Option<Self> {
        match value {
            raw_model::ActivationBarrierStatus::Unspecified => None,
            raw_model::ActivationBarrierStatus::NotDeclared => Some(Self::NotDeclared),
            raw_model::ActivationBarrierStatus::Pending => Some(Self::Pending),
            raw_model::ActivationBarrierStatus::Cleared => Some(Self::Cleared),
        }
    }
}

/// Which evidence basis justifies an authoritative conclusion.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum SettlementBasisKind {
    /// The full required evidence set is closed at settlement.
    CompleteEvidence,
    /// A named proof rule shows a witnessed violation is outcome invariant.
    DecisiveViolation,
    /// A named proof rule shows watermark-complete silence is outcome invariant.
    WatermarkAnchoredViolation,
}

/// Read-only view of one recorded source sequence gap.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct SourceGapView {
    /// Source whose sequence is discontinuous.
    pub source: SourceIdentity,
    /// Sequence epoch the gap falls in.
    pub sequence_epoch: u64,
    /// First missing sequence number.
    pub first_missing: u64,
    /// Last missing sequence number.
    pub last_missing: u64,
    /// Effect the gap has on the conclusion.
    pub effect: String,
}

/// Read-only view of one declared uncertainty.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct UncertaintyView {
    /// Named reason the evidence is open.
    pub reason: InconclusiveReason,
    /// Concrete evidence the reason refers to.
    pub detail: String,
}

/// Read-only view of the assurance downgrade for missing anchor evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct SourceTierGapView {
    /// Covered required sources classified as contributing tier.
    pub contributing_sources: BTreeSet<SourceIdentity>,
    /// Required sources without valid anchor evidence.
    pub unanchored_sources: BTreeSet<SourceIdentity>,
    /// Why the conclusion rests on no anchor evidence.
    pub detail: String,
}

fn domain_sources(sources: &[raw_model::SourceIdentity]) -> BTreeSet<SourceIdentity> {
    sources
        .iter()
        .map(|source| source_from_wire(source).expect("validated source"))
        .collect()
}

/// Validated Verdict backed only by generated storage.
#[derive(Clone, Debug, PartialEq)]
pub struct Verdict(raw_model::Verdict);

impl Verdict {
    /// Encodes the canonical Verdict in Protobuf wire format.
    #[must_use]
    pub fn encode_to_vec(&self) -> Vec<u8> {
        self.0.encode_to_vec()
    }

    /// Encodes an unvalidated wire Verdict.
    ///
    /// Store conformance uses this to inject pre-#164 / pre-#206 `bytea` whose field 22 is
    /// proto3-default `UNSPECIFIED` and whose retention/redaction fields 24–28 are absent.
    /// [`Self::try_from`] rejects those shapes; [`Self::decode`] recovers them.
    #[must_use]
    pub fn encode_unvalidated(value: &raw_model::Verdict) -> Vec<u8> {
        value.encode_to_vec()
    }

    /// Decodes and validates an untrusted wire message.
    ///
    /// Pre-#164 payloads have proto3-default `UNSPECIFIED` on field 22. Pre-#206
    /// payloads omit retention/redaction fields 24–28. This entry relabels / backfills
    /// them before [`TryFrom`]: that is the ADR-0011 agreement both store backends
    /// share. [`TryFrom`] itself does **not** recover — a hand-built absence fails.
    /// On-disk `bytea` stays absent forever; AC4 is a codec, not a SQL data migration.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed bytes or a decoded message that
    /// violates a canonical Verdict invariant.
    pub fn decode(bytes: &[u8]) -> Result<Self, VerdictError> {
        let mut raw = raw_model::Verdict::decode(bytes)
            .map_err(|error| VerdictError::Decode(error.to_string()))?;
        if let Some(assurance) = raw.assurance.as_mut() {
            relabel_legacy_activation_barrier(assurance);
            backfill_legacy_retention(assurance);
        }
        Self::try_from(raw)
    }

    fn assurance(&self) -> &raw_model::AssuranceEnvelope {
        self.0.assurance.as_ref().expect("validated assurance")
    }

    /// Returns the Verdict identity.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.0.id
    }

    /// Returns the conclusion reached.
    #[must_use]
    pub fn outcome(&self) -> VerdictOutcome {
        VerdictOutcome::from_raw(
            raw_model::VerdictOutcome::try_from(self.0.outcome).expect("validated outcome"),
        )
        .expect("validated outcome")
    }

    /// Returns the named reason an Inconclusive conclusion could not settle.
    #[must_use]
    pub fn inconclusive_reason(&self) -> Option<InconclusiveReason> {
        self.0.inconclusive_reason.map(|reason| {
            InconclusiveReason::from_raw(
                raw_model::InconclusiveReason::try_from(reason).expect("validated reason"),
            )
            .expect("validated reason")
        })
    }

    /// Returns how much authority the conclusion claims.
    #[must_use]
    pub fn authority(&self) -> VerdictAuthority {
        VerdictAuthority::from_raw(
            raw_model::VerdictAuthority::try_from(self.0.authority).expect("validated authority"),
        )
        .expect("validated authority")
    }

    /// Returns the evaluated Specification identity.
    #[must_use]
    pub fn specification_id(&self) -> &str {
        &self.assurance().specification_id
    }

    /// Returns the evaluated Specification version.
    #[must_use]
    pub fn specification_version(&self) -> &str {
        &self.assurance().specification_version
    }

    /// Returns the sources the Specification requires.
    #[must_use]
    pub fn required_sources(&self) -> BTreeSet<SourceIdentity> {
        domain_sources(&self.assurance().required_sources)
    }

    /// Returns the required sources that supplied evidence.
    #[must_use]
    pub fn observed_sources(&self) -> BTreeSet<SourceIdentity> {
        domain_sources(&self.assurance().observed_sources)
    }

    /// Returns recorded source sequence gaps in canonical order.
    #[must_use]
    pub fn gaps(&self) -> Vec<SourceGapView> {
        self.assurance()
            .gaps
            .iter()
            .map(|gap| SourceGapView {
                source: source_from_wire(gap.source.as_ref().expect("validated gap source"))
                    .expect("validated gap source"),
                sequence_epoch: gap.sequence_epoch,
                first_missing: gap.first_missing,
                last_missing: gap.last_missing,
                effect: gap.effect.clone(),
            })
            .collect()
    }

    /// Returns declared uncertainty in canonical order, at most one entry per reason.
    #[must_use]
    pub fn uncertainty(&self) -> Vec<UncertaintyView> {
        self.assurance()
            .uncertainty
            .iter()
            .map(|uncertainty| UncertaintyView {
                reason: InconclusiveReason::from_raw(
                    raw_model::InconclusiveReason::try_from(uncertainty.reason)
                        .expect("validated reason"),
                )
                .expect("validated reason"),
                detail: uncertainty.detail.clone(),
            })
            .collect()
    }

    /// Returns hot-path settlement state.
    #[must_use]
    pub fn hot_path(&self) -> TierSettlement {
        TierSettlement::from_raw(
            raw_model::TierSettlement::try_from(self.assurance().hot_path)
                .expect("validated hot path"),
        )
        .expect("validated hot path")
    }

    /// Returns cold-path settlement state.
    #[must_use]
    pub fn cold_path(&self) -> TierSettlement {
        TierSettlement::from_raw(
            raw_model::TierSettlement::try_from(self.assurance().cold_path)
                .expect("validated cold path"),
        )
        .expect("validated cold path")
    }

    /// Returns which evidence basis justifies the conclusion, if any.
    #[must_use]
    pub fn settlement_basis(&self) -> Option<SettlementBasisKind> {
        match self
            .assurance()
            .settlement_basis
            .as_ref()
            .and_then(|basis| basis.basis.as_ref())?
        {
            raw_model::settlement_basis::Basis::CompleteEvidence(_) => {
                Some(SettlementBasisKind::CompleteEvidence)
            }
            raw_model::settlement_basis::Basis::DecisiveViolation(_) => {
                Some(SettlementBasisKind::DecisiveViolation)
            }
            raw_model::settlement_basis::Basis::WatermarkAnchoredViolation(_) => {
                Some(SettlementBasisKind::WatermarkAnchoredViolation)
            }
        }
    }

    /// Returns the assurance downgrade recorded when no covered source anchors evidence.
    #[must_use]
    pub fn source_tier_gap(&self) -> Option<SourceTierGapView> {
        self.assurance()
            .source_tier_gap
            .as_ref()
            .map(|gap| SourceTierGapView {
                contributing_sources: domain_sources(&gap.contributing_sources),
                unanchored_sources: domain_sources(&gap.unanchored_sources),
                detail: gap.detail.clone(),
            })
    }

    /// Returns the anchor-tier Event identities the conclusion rests on, per source.
    #[must_use]
    pub fn anchored_event_ids(&self) -> BTreeMap<SourceIdentity, BTreeSet<EventId>> {
        self.assurance()
            .source_tiers_and_anchors
            .iter()
            .map(|record| {
                let source =
                    source_from_wire(record.source.as_ref().expect("validated tier source"))
                        .expect("validated tier source");
                let events = record
                    .anchored_event_ids
                    .iter()
                    .map(|id| EventId::new(id.clone()).expect("validated anchored Event id"))
                    .collect();
                (source, events)
            })
            .collect()
    }

    /// Returns the Event identities named by a decisive violation witness.
    #[must_use]
    pub fn decisive_witness_event_ids(&self) -> BTreeSet<EventId> {
        let Some(raw_model::settlement_basis::Basis::DecisiveViolation(basis)) = self
            .assurance()
            .settlement_basis
            .as_ref()
            .and_then(|basis| basis.basis.as_ref())
        else {
            return BTreeSet::new();
        };
        basis
            .witness_events
            .iter()
            .map(|event| EventId::new(event.event_id.clone()).expect("validated witness Event id"))
            .collect()
    }

    /// Returns the uncertainty reasons a decisive violation proof declares as invariant.
    #[must_use]
    pub fn outcome_invariant_uncertainty(&self) -> BTreeSet<InconclusiveReason> {
        let Some(raw_model::settlement_basis::Basis::DecisiveViolation(basis)) = self
            .assurance()
            .settlement_basis
            .as_ref()
            .and_then(|basis| basis.basis.as_ref())
        else {
            return BTreeSet::new();
        };
        basis
            .outcome_invariant_limitations
            .as_ref()
            .map(|limitations| {
                limitations
                    .uncertainty
                    .iter()
                    .map(|reason| {
                        InconclusiveReason::from_raw(
                            raw_model::InconclusiveReason::try_from(*reason)
                                .expect("validated reason"),
                        )
                        .expect("validated reason")
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Returns the Event Filter epoch each required source acknowledged.
    ///
    /// Acknowledged-only: rows whose `filter_epoch` is unset are omitted. Silent
    /// required sources holding a pending barrier appear in [`Self::filter_epoch_rows`]
    /// with `None`, not here. The Specification-level target is
    /// [`Self::target_filter_epoch`].
    #[must_use]
    pub fn filter_epochs(&self) -> BTreeMap<SourceIdentity, FilterEpoch> {
        self.filter_epoch_rows()
            .into_iter()
            .filter_map(|(source, epoch)| Some((source, epoch?)))
            .collect()
    }

    /// Returns every `filter_epochs` row, including sources whose acknowledged epoch is unset.
    ///
    /// Silent-source tests must use this, not [`Self::filter_epochs`]: the acknowledged-only
    /// map drops the write those tests exist to pin.
    #[must_use]
    pub fn filter_epoch_rows(&self) -> BTreeMap<SourceIdentity, Option<FilterEpoch>> {
        self.assurance()
            .filter_epochs
            .iter()
            .filter_map(|entry| {
                let source = source_from_wire(entry.source.as_ref()?).ok()?;
                Some((source, entry.filter_epoch.map(FilterEpoch::new)))
            })
            .collect()
    }

    /// Returns the typed activation-barrier discriminator.
    ///
    /// `UNSPECIFIED` is not constructible: [`Self::try_from`] rejects it and
    /// [`Self::decode`] relabels historical blobs first.
    #[must_use]
    pub fn activation_barrier(&self) -> ActivationBarrierStatus {
        ActivationBarrierStatus::from_raw(
            raw_model::ActivationBarrierStatus::try_from(self.assurance().activation_barrier)
                .expect("validated activation barrier"),
        )
        .expect("validated activation barrier")
    }

    /// Returns the Specification-level target Filter epoch, when one is on the envelope.
    ///
    /// Unset on [`ActivationBarrierStatus::NotDeclared`] and on the historical #156
    /// `PENDING` residual (`NoEvidence` / non-`FilterEpochUnacknowledged` rows that never
    /// interpolated a target). Do not treat epoch `0` as missing: it is a real epoch.
    #[must_use]
    pub fn target_filter_epoch(&self) -> Option<FilterEpoch> {
        self.assurance().target_filter_epoch.map(FilterEpoch::new)
    }

    /// Returns the raw-Event retention horizon recorded at settlement (ADR-0026 §6).
    #[must_use]
    pub fn raw_event_retention_days(&self) -> RawEventRetentionDays {
        RawEventRetentionDays::try_new(
            self.assurance()
                .raw_event_retention_days
                .expect("validated raw-Event retention"),
        )
        .expect("validated raw-Event retention")
    }

    /// Returns the evidence-extract retention claim recorded at settlement.
    #[must_use]
    pub fn evidence_extract_retention(&self) -> RetentionWindow {
        match self
            .assurance()
            .evidence_extract_retention
            .as_ref()
            .expect("validated evidence-extract retention")
        {
            raw_model::assurance_envelope::EvidenceExtractRetention::EvidenceExtractRetentionDays(
                days,
            ) => RetentionWindow::Days(
                EvidenceExtractRetentionDays::try_new(*days)
                    .expect("validated evidence-extract days"),
            ),
            raw_model::assurance_envelope::EvidenceExtractRetention::EvidenceExtractNotDeployed(
                _,
            ) => RetentionWindow::NotDeployed,
        }
    }

    /// Returns the redaction-policy claim recorded at settlement (ADR-0026 §7).
    #[must_use]
    pub fn redaction_policy(&self) -> RedactionPolicyVersion {
        match self
            .assurance()
            .redaction_policy
            .as_ref()
            .expect("validated redaction policy")
        {
            raw_model::assurance_envelope::RedactionPolicy::RedactionPolicyVersion(version) => {
                RedactionPolicyVersion::Version(version.clone())
            }
            raw_model::assurance_envelope::RedactionPolicy::RedactionPolicyNone(_) => {
                RedactionPolicyVersion::NoPolicyConfigured
            }
        }
    }

    /// Returns the forecast-at-settle redacted-attribute inventory (ADR-0026 §7; #244).
    ///
    /// Empty means only that no claims are recorded on this envelope — not that
    /// redaction completed, was a noop, or that a worker has run. Paths are opaque
    /// Tenant-policy keys. Entries are in canonical `(event_id, attribute_path)` order.
    #[must_use]
    pub fn redacted_attributes(&self) -> Vec<RedactedAttributeRef> {
        self.assurance()
            .redacted_attributes
            .iter()
            .map(|entry| RedactedAttributeRef {
                event_id: entry.event_id.clone(),
                attribute_path: entry.attribute_path.clone(),
            })
            .collect()
    }

    /// Returns per-source completeness watermarks.
    #[must_use]
    pub fn watermarks(&self) -> BTreeMap<SourceIdentity, u64> {
        watermark_view(&self.assurance().watermarks)
    }

    /// Returns per-source hot-path settlement watermarks.
    #[must_use]
    pub fn hot_settlement_watermarks(&self) -> BTreeMap<SourceIdentity, u64> {
        watermark_view(&self.assurance().hot_settlement_watermarks)
    }

    /// Returns per-source cold-path settlement watermarks.
    #[must_use]
    pub fn cold_settlement_watermarks(&self) -> BTreeMap<SourceIdentity, u64> {
        watermark_view(&self.assurance().cold_settlement_watermarks)
    }

    /// Returns declared causal edges whose predecessor was never admitted.
    #[must_use]
    pub fn unresolved_causal_edges(&self) -> BTreeSet<(EventId, EventId)> {
        self.assurance()
            .unresolved_causal_edges
            .iter()
            .map(|edge| {
                (
                    EventId::new(edge.event_id.clone()).expect("validated edge Event id"),
                    EventId::new(edge.missing_predecessor_id.clone())
                        .expect("validated missing predecessor id"),
                )
            })
            .collect()
    }

    /// Returns the sources reported unhealthy.
    #[must_use]
    pub fn unhealthy_sources(&self) -> BTreeSet<SourceIdentity> {
        self.assurance()
            .source_health
            .iter()
            .filter(|record| !record.healthy)
            .map(|record| {
                source_from_wire(record.source.as_ref().expect("validated health source"))
                    .expect("validated health source")
            })
            .collect()
    }

    /// Returns effective capabilities per source.
    #[must_use]
    pub fn capabilities(&self) -> BTreeMap<SourceIdentity, SourceCapabilities> {
        self.assurance()
            .capabilities
            .iter()
            .map(|record| {
                let source =
                    source_from_wire(record.source.as_ref().expect("validated capability source"))
                        .expect("validated capability source");
                let capabilities = capabilities_from_wire(&record.capabilities);
                (source, capabilities)
            })
            .collect()
    }
}

fn watermark_view(watermarks: &[raw_model::Watermark]) -> BTreeMap<SourceIdentity, u64> {
    watermarks
        .iter()
        .map(|watermark| {
            let source = source_from_wire(
                watermark
                    .source
                    .as_ref()
                    .expect("validated watermark source"),
            )
            .expect("validated watermark source");
            (source, watermark.complete_through)
        })
        .collect()
}

fn capabilities_from_wire(values: &[i32]) -> SourceCapabilities {
    SourceCapabilities::new(values.iter().map(|value| {
        match raw_model::SourceCapability::try_from(*value).expect("validated capability") {
            raw_model::SourceCapability::Unspecified => {
                unreachable!("validated capability is never unspecified")
            }
            raw_model::SourceCapability::ObserveBeforeEffect => {
                SourceCapability::ObserveBeforeEffect
            }
            raw_model::SourceCapability::CausalEdges => SourceCapability::CausalEdges,
            raw_model::SourceCapability::CompletenessSignals => {
                SourceCapability::CompletenessSignals
            }
            raw_model::SourceCapability::BoundedClockUncertainty => {
                SourceCapability::BoundedClockUncertainty
            }
        }
    }))
}

/// Relabels a pre-#164 envelope that has no typed barrier discriminator.
///
/// Applied only by [`Verdict::decode`], never by [`TryFrom`]. Idempotent: a blob whose
/// field 22 is already set is unchanged, including a buggy new-Core blob — the PENDING
/// validator is not what forces Core to write the target.
///
/// Historical warming rows stay `cold_path = PENDING` with empty cold watermarks: the
/// discarded coordinate is unrecoverable (limitation 10(b)). Historical `NoEvidence` /
/// non-`FilterEpochUnacknowledged` rows may never learn a target; do not invent one and
/// do not use `0`. Historical `CLEARED` cannot be recovered either (#156 never emitted
/// `Authoritative` while warming; a cleared barrier was byte-identical to undeclared),
/// so those rows stay `NOT_DECLARED`.
fn relabel_legacy_activation_barrier(assurance: &mut raw_model::AssuranceEnvelope) {
    let already_set = raw_model::ActivationBarrierStatus::try_from(assurance.activation_barrier)
        .is_ok_and(|status| status != raw_model::ActivationBarrierStatus::Unspecified);
    if already_set {
        return;
    }

    let pending = assurance
        .uncertainty
        .iter()
        .any(|entry| entry.detail.contains(BARRIER_PENDING_DETAIL_PREFIX));
    if !pending {
        assurance.activation_barrier = raw_model::ActivationBarrierStatus::NotDeclared as i32;
        assurance.target_filter_epoch = None;
        return;
    }

    assurance.activation_barrier = raw_model::ActivationBarrierStatus::Pending as i32;
    let targets = collected_legacy_target_epochs(
        assurance
            .uncertainty
            .iter()
            .map(|entry| entry.detail.as_str()),
    );
    if targets.len() == 1 {
        assurance.target_filter_epoch = targets.into_iter().next();
    } else {
        assurance.target_filter_epoch = None;
    }
    insert_silent_required_source_rows(assurance);
}

/// Backfills pre-#206 envelopes that omit retention/redaction fields 24–28.
///
/// Applied only by [`Verdict::decode`], never by [`TryFrom`]. Idempotent: arms already
/// set are unchanged. Historical recovery uses the platform raw default of 30,
/// evidence-extract [`RetentionWindow::NotDeployed`], and
/// [`RedactionPolicyVersion::NoPolicyConfigured`] — not an export default for new
/// construction. On-disk `bytea` stays absent forever; no SQL rewrite.
fn backfill_legacy_retention(assurance: &mut raw_model::AssuranceEnvelope) {
    if assurance.raw_event_retention_days.is_none() {
        assurance.raw_event_retention_days = Some(PLATFORM_DEFAULT_RAW_EVENT_RETENTION_DAYS);
    }
    if assurance.evidence_extract_retention.is_none() {
        assurance.evidence_extract_retention = Some(
            raw_model::assurance_envelope::EvidenceExtractRetention::EvidenceExtractNotDeployed(
                raw_model::EvidenceExtractRetentionNotDeployed {},
            ),
        );
    }
    if assurance.redaction_policy.is_none() {
        assurance.redaction_policy = Some(
            raw_model::assurance_envelope::RedactionPolicy::RedactionPolicyNone(
                raw_model::RedactionPolicyNone {},
            ),
        );
    }
}

const LEGACY_TARGET_PHRASE: &str = "below the target epoch ";

fn collected_legacy_target_epochs<'a>(details: impl Iterator<Item = &'a str>) -> BTreeSet<u64> {
    let mut found = BTreeSet::new();
    for detail in details {
        let mut rest = detail;
        while let Some(index) = rest.find(LEGACY_TARGET_PHRASE) {
            rest = &rest[index + LEGACY_TARGET_PHRASE.len()..];
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            if let Ok(target) = digits.parse::<u64>() {
                found.insert(target);
            }
        }
    }
    found
}

fn insert_silent_required_source_rows(assurance: &mut raw_model::AssuranceEnvelope) {
    let present: BTreeSet<SourceKey> = assurance
        .filter_epochs
        .iter()
        .filter_map(|row| row.source.as_ref().map(owned_source_key))
        .collect();
    for source in &assurance.required_sources {
        if !present.contains(&owned_source_key(source)) {
            assurance
                .filter_epochs
                .push(raw_model::FilterEpochBySource {
                    source: Some(source.clone()),
                    filter_epoch: None,
                });
        }
    }
}

impl TryFrom<raw_model::Verdict> for Verdict {
    type Error = VerdictError;

    fn try_from(mut value: raw_model::Verdict) -> Result<Self, Self::Error> {
        require_text(&value.id, "verdict.id")?;
        let outcome = raw_model::VerdictOutcome::try_from(value.outcome).map_err(|_| {
            VerdictError::UnknownEnum {
                field: "verdict.outcome",
                value: value.outcome,
            }
        })?;
        if outcome == raw_model::VerdictOutcome::Unspecified {
            return Err(VerdictError::UnspecifiedEnum("verdict.outcome"));
        }
        let authority = raw_model::VerdictAuthority::try_from(value.authority).map_err(|_| {
            VerdictError::UnknownEnum {
                field: "verdict.authority",
                value: value.authority,
            }
        })?;
        if authority == raw_model::VerdictAuthority::Unspecified {
            return Err(VerdictError::UnspecifiedEnum("verdict.authority"));
        }
        match (outcome, value.inconclusive_reason) {
            (raw_model::VerdictOutcome::Inconclusive, Some(reason)) => {
                match raw_model::InconclusiveReason::try_from(reason) {
                    Ok(raw_model::InconclusiveReason::Unspecified) | Err(_) => {
                        return Err(VerdictError::VerdictReasonMismatch)
                    }
                    Ok(_) => {}
                }
            }
            (raw_model::VerdictOutcome::Inconclusive, None) | (_, Some(_)) => {
                return Err(VerdictError::VerdictReasonMismatch)
            }
            (_, None) => {}
        }
        let assurance = value
            .assurance
            .as_ref()
            .ok_or(VerdictError::MissingField("verdict.assurance"))?;
        AssuranceValidator::check(assurance, outcome, authority)?;
        canonicalize_assurance(
            value
                .assurance
                .as_mut()
                .expect("assurance was validated as present"),
        );
        Ok(Self(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `VerdictError` variant paired with the exact `Display` output it must produce.
    ///
    /// Carried over verbatim from the single-error table this type was split out of (#65).
    #[allow(clippy::too_many_lines)] // One entry per variant; the exhaustive table is the point.
    fn error_display_cases() -> Vec<(VerdictError, &'static str)> {
        let wrapped = crate::EventError::SelfCausalPredecessor;
        vec![
            (
                VerdictError::MissingScalar("field"),
                "required scalar is missing: field",
            ),
            (
                VerdictError::MissingField("field"),
                "required message is missing: field",
            ),
            (
                VerdictError::UnknownEnum {
                    field: "field",
                    value: 7,
                },
                "unknown field enum value: 7",
            ),
            (
                VerdictError::UnspecifiedEnum("field"),
                "field must not be unspecified",
            ),
            (
                VerdictError::DuplicateValue("field"),
                "duplicate value in set-like field: field",
            ),
            (
                VerdictError::VerdictReasonMismatch,
                "Verdict outcome and inconclusive reason disagree",
            ),
            (
                VerdictError::AuthoritativeTierUnsettled,
                "authoritative Verdict requires all required tiers to settle",
            ),
            (
                VerdictError::ProvisionalTierMismatch,
                "provisional Verdict requires a settled hot path and either a pending cold path or a pending activation barrier",
            ),
            (
                VerdictError::AuthoritativeBarrierPending,
                "authoritative Verdict cannot carry a pending activation barrier",
            ),
            (
                VerdictError::ClearedBarrierIncomplete,
                "cleared activation barrier requires a target Filter epoch and an acknowledged epoch at or above it on every required source",
            ),
            (
                VerdictError::UndeclaredBarrierHasTarget,
                "undeclared activation barrier must not carry a target Filter epoch",
            ),
            (
                VerdictError::AssuranceSourceMismatch("field"),
                "assurance source is not required: field",
            ),
            (
                VerdictError::AuthoritativeCoverageIncomplete("field"),
                "authoritative assurance lacks required-source coverage: field",
            ),
            (
                VerdictError::InvalidAnchorEvidence,
                "source tier and anchored Event evidence disagree",
            ),
            (
                VerdictError::SourceTierGapMismatch,
                "source-tier gap does not match contributing-only evidence",
            ),
            (
                VerdictError::MissingAnchorEvidence,
                "authoritative conclusion requires valid anchor-tier evidence",
            ),
            (
                VerdictError::MissingSettlementBasis,
                "authoritative conclusion requires an outcome settlement basis",
            ),
            (
                VerdictError::SettlementBasisMismatch,
                "settlement basis does not justify Verdict outcome or authority",
            ),
            (
                VerdictError::AuthoritativeEvidenceIncomplete("field"),
                "authoritative evidence remains open: field",
            ),
            (
                VerdictError::SettlementEvidenceIncomplete("field"),
                "evidence does not reach settlement Watermark: field",
            ),
            (
                VerdictError::InvalidDecisiveWitness("field"),
                "invalid decisive violation witness: field",
            ),
            (
                VerdictError::InvariantLimitationsMismatch("field"),
                "outcome-invariant limitations disagree: field",
            ),
            (
                VerdictError::SettlementWatermarkMismatch("field"),
                "tier settlement and Watermarks disagree: field",
            ),
            (
                VerdictError::InvalidEvidence(wrapped),
                "invalid model evidence: event must not be its own causal predecessor",
            ),
            (
                VerdictError::InvalidRange("field"),
                "invalid numeric range: field",
            ),
            (
                VerdictError::RedactedAttributesWithoutPolicy,
                "non-empty redacted_attributes requires a configured redaction_policy_version",
            ),
            (
                VerdictError::Decode("field".to_owned()),
                "invalid protobuf bytes: field",
            ),
        ]
    }

    #[test]
    fn error_display_is_stable() {
        let cases = error_display_cases();
        assert_eq!(cases.len(), 27, "every VerdictError variant must be pinned");
        for (error, expected) in cases {
            assert_eq!(error.to_string(), expected, "{error:?}");
        }
    }

    #[test]
    fn error_source_is_exposed_only_for_wrapped_errors() {
        for (error, _) in error_display_cases() {
            let wraps_event_error = matches!(error, VerdictError::InvalidEvidence(_));
            assert_eq!(error.source().is_some(), wraps_event_error, "{error:?}");
        }
    }

    fn source() -> raw_model::SourceIdentity {
        raw_model::SourceIdentity {
            monitored_application_id: "app".to_owned(),
            source_id: "gateway".to_owned(),
        }
    }

    fn source_named(source_id: &str) -> raw_model::SourceIdentity {
        raw_model::SourceIdentity {
            monitored_application_id: "app".to_owned(),
            source_id: source_id.to_owned(),
        }
    }

    fn observed_at() -> raw_model::ObservationTime {
        raw_model::ObservationTime {
            sentinel_clock_id: "sentinel".to_owned(),
            nanoseconds: Some(raw_model::Int128::default()),
        }
    }

    fn watermark() -> raw_model::Watermark {
        raw_model::Watermark {
            source: Some(source()),
            sequence_epoch: 1,
            complete_through: 4,
            observed_at: Some(observed_at()),
        }
    }

    fn assurance() -> raw_model::AssuranceEnvelope {
        raw_model::AssuranceEnvelope {
            specification_id: "spec".to_owned(),
            specification_version: "v1".to_owned(),
            schema_versions: vec!["event.v1".to_owned()],
            filter_epochs: vec![raw_model::FilterEpochBySource {
                source: Some(source()),
                filter_epoch: Some(9),
            }],
            watermarks: vec![watermark()],
            required_sources: vec![source()],
            observed_sources: vec![source()],
            gaps: Vec::new(),
            source_health: vec![raw_model::SourceHealth {
                source: Some(source()),
                healthy: true,
                detail: String::new(),
            }],
            capabilities: vec![raw_model::SourceCapabilityRecord {
                source: Some(source()),
                capabilities: vec![raw_model::SourceCapability::CausalEdges as i32],
            }],
            source_tiers_and_anchors: vec![raw_model::SourceTierRecord {
                source: Some(source()),
                tier: raw_model::SourceTier::Anchor as i32,
                anchored_event_ids: vec!["event".to_owned()],
            }],
            unresolved_causal_edges: Vec::new(),
            extraction_facts: vec![raw_model::ExtractionFact {
                fact_id: "amount".to_owned(),
                available: true,
                detail: String::new(),
            }],
            uncertainty: Vec::new(),
            late_data_policy: raw_model::LateDataPolicy::Exclude as i32,
            hot_path: raw_model::TierSettlement::Settled as i32,
            cold_path: raw_model::TierSettlement::NotRequired as i32,
            hot_settlement_watermarks: vec![watermark()],
            cold_settlement_watermarks: Vec::new(),
            source_tier_gap: None,
            settlement_basis: Some(raw_model::SettlementBasis {
                basis: Some(raw_model::settlement_basis::Basis::CompleteEvidence(
                    raw_model::CompleteEvidenceBasis {},
                )),
            }),
            activation_barrier: raw_model::ActivationBarrierStatus::NotDeclared as i32,
            target_filter_epoch: None,
            raw_event_retention_days: Some(PLATFORM_DEFAULT_RAW_EVENT_RETENTION_DAYS),
            evidence_extract_retention: Some(
                raw_model::assurance_envelope::EvidenceExtractRetention::EvidenceExtractNotDeployed(
                    raw_model::EvidenceExtractRetentionNotDeployed {},
                ),
            ),
            redaction_policy: Some(
                raw_model::assurance_envelope::RedactionPolicy::RedactionPolicyNone(
                    raw_model::RedactionPolicyNone {},
                ),
            ),
            redacted_attributes: Vec::new(),
        }
    }

    fn verdict(assurance: raw_model::AssuranceEnvelope) -> raw_model::Verdict {
        raw_model::Verdict {
            id: "verdict".to_owned(),
            outcome: raw_model::VerdictOutcome::Satisfy as i32,
            inconclusive_reason: None,
            authority: raw_model::VerdictAuthority::Authoritative as i32,
            assurance: Some(assurance),
        }
    }

    fn decisive_violation_basis(
        assurance: &raw_model::AssuranceEnvelope,
    ) -> raw_model::SettlementBasis {
        raw_model::SettlementBasis {
            basis: Some(raw_model::settlement_basis::Basis::DecisiveViolation(
                raw_model::DecisiveViolationBasis {
                    witness_events: vec![raw_model::DecisiveEvent {
                        event_id: "event".to_owned(),
                        source: Some(source()),
                        sequence_epoch: Some(1),
                        sequence_number: Some(4),
                    }],
                    required_extraction_fact_ids: vec!["amount".to_owned()],
                    invariance_proof_rule_id: "existence-witness".to_owned(),
                    invariance_proof_rule_version: "v1".to_owned(),
                    outcome_invariant_limitations: Some(raw_model::OutcomeInvariantLimitations {
                        gaps: assurance.gaps.clone(),
                        unhealthy_sources: assurance
                            .source_health
                            .iter()
                            .filter(|health| !health.healthy)
                            .filter_map(|health| health.source.clone())
                            .collect(),
                        unresolved_causal_edges: assurance.unresolved_causal_edges.clone(),
                        unavailable_extraction_fact_ids: assurance
                            .extraction_facts
                            .iter()
                            .filter(|fact| !fact.available)
                            .map(|fact| fact.fact_id.clone())
                            .collect(),
                        uncertainty: assurance
                            .uncertainty
                            .iter()
                            .map(|uncertainty| uncertainty.reason)
                            .collect(),
                    }),
                },
            )),
        }
    }

    fn watermark_anchored_violation_basis(
        assurance: &raw_model::AssuranceEnvelope,
    ) -> raw_model::SettlementBasis {
        raw_model::SettlementBasis {
            basis: Some(
                raw_model::settlement_basis::Basis::WatermarkAnchoredViolation(
                    raw_model::WatermarkAnchoredViolationBasis {
                        silent_anchor_sources: assurance.required_sources.clone(),
                        silence_through_watermarks: assurance.watermarks.clone(),
                        invariance_proof_rule_id: "watermark-silence".to_owned(),
                        invariance_proof_rule_version: "v1".to_owned(),
                        outcome_invariant_limitations: Some(
                            raw_model::OutcomeInvariantLimitations {
                                gaps: assurance.gaps.clone(),
                                unhealthy_sources: assurance
                                    .source_health
                                    .iter()
                                    .filter(|health| !health.healthy)
                                    .filter_map(|health| health.source.clone())
                                    .collect(),
                                unresolved_causal_edges: assurance.unresolved_causal_edges.clone(),
                                unavailable_extraction_fact_ids: assurance
                                    .extraction_facts
                                    .iter()
                                    .filter(|fact| !fact.available)
                                    .map(|fact| fact.fact_id.clone())
                                    .collect(),
                                uncertainty: assurance
                                    .uncertainty
                                    .iter()
                                    .map(|uncertainty| uncertainty.reason)
                                    .collect(),
                            },
                        ),
                    },
                ),
            ),
        }
    }

    fn watermark_anchored_violate(
        mut envelope: raw_model::AssuranceEnvelope,
    ) -> raw_model::Verdict {
        envelope.source_tiers_and_anchors[0]
            .anchored_event_ids
            .clear();
        envelope.settlement_basis = Some(watermark_anchored_violation_basis(&envelope));
        let mut violation = verdict(envelope);
        violation.outcome = raw_model::VerdictOutcome::Violate as i32;
        violation
    }

    #[test]
    fn watermark_anchored_silence_can_authoritatively_violate() {
        let settled = Verdict::try_from(watermark_anchored_violate(assurance()))
            .expect("ADR-0033 silence is a Violate");
        assert_eq!(settled.outcome(), VerdictOutcome::Violate);
        assert_eq!(
            settled.settlement_basis(),
            Some(SettlementBasisKind::WatermarkAnchoredViolation)
        );
        assert_eq!(settled.source_tier_gap(), None);
    }

    #[test]
    fn watermark_anchored_violation_rejects_invalid_silence() {
        let mut empty_silent = assurance();
        let mut basis = watermark_anchored_violation_basis(&empty_silent);
        let Some(raw_model::settlement_basis::Basis::WatermarkAnchoredViolation(inner)) =
            basis.basis.as_mut()
        else {
            panic!("watermark fixture must contain watermark basis");
        };
        inner.silent_anchor_sources.clear();
        inner.silence_through_watermarks.clear();
        empty_silent.settlement_basis = Some(basis);
        let mut empty_silent = verdict(empty_silent);
        empty_silent.outcome = raw_model::VerdictOutcome::Violate as i32;
        assert_eq!(
            Verdict::try_from(empty_silent),
            Err(VerdictError::MissingField(
                "assurance.settlement_basis.silent_anchor_sources"
            ))
        );

        let mut contributing = assurance();
        contributing.source_tiers_and_anchors[0].tier = raw_model::SourceTier::Contributing as i32;
        assert_eq!(
            Verdict::try_from(watermark_anchored_violate(contributing)),
            Err(VerdictError::InvalidDecisiveWitness(
                "silent_anchor_sources.tier"
            ))
        );

        let mut unhealthy = assurance();
        unhealthy.source_health[0].healthy = false;
        unhealthy.source_health[0].detail = "heartbeat expired".to_owned();
        assert_eq!(
            Verdict::try_from(watermark_anchored_violate(unhealthy)),
            Err(VerdictError::InvalidDecisiveWitness(
                "silent_anchor_sources.health"
            ))
        );

        let mut still_anchored = assurance();
        still_anchored.settlement_basis = Some(watermark_anchored_violation_basis(&still_anchored));
        let mut still_anchored = verdict(still_anchored);
        still_anchored.outcome = raw_model::VerdictOutcome::Violate as i32;
        assert_eq!(
            Verdict::try_from(still_anchored),
            Err(VerdictError::InvalidDecisiveWitness(
                "silent_anchor_sources.anchored_event_ids"
            ))
        );

        let mut mismatched = assurance();
        mismatched.source_tiers_and_anchors[0]
            .anchored_event_ids
            .clear();
        let mut basis = watermark_anchored_violation_basis(&mismatched);
        let Some(raw_model::settlement_basis::Basis::WatermarkAnchoredViolation(inner)) =
            basis.basis.as_mut()
        else {
            panic!("watermark fixture must contain watermark basis");
        };
        inner.silence_through_watermarks.clear();
        mismatched.settlement_basis = Some(basis);
        let mut mismatched = verdict(mismatched);
        mismatched.outcome = raw_model::VerdictOutcome::Violate as i32;
        assert_eq!(
            Verdict::try_from(mismatched),
            Err(VerdictError::InvalidDecisiveWitness(
                "silence_through_watermarks"
            ))
        );

        let mut surplus_gap = assurance();
        surplus_gap.source_tiers_and_anchors[0]
            .anchored_event_ids
            .clear();
        surplus_gap.settlement_basis = Some(watermark_anchored_violation_basis(&surplus_gap));
        surplus_gap.source_tier_gap = Some(raw_model::SourceTierGap {
            contributing_sources: Vec::new(),
            detail: "the conclusion rests on no anchor-tier Event".to_owned(),
            unanchored_sources: vec![source()],
        });
        let mut surplus_gap = verdict(surplus_gap);
        surplus_gap.outcome = raw_model::VerdictOutcome::Violate as i32;
        assert_eq!(
            Verdict::try_from(surplus_gap),
            Err(VerdictError::SourceTierGapMismatch)
        );

        let mut satisfy = assurance();
        satisfy.source_tiers_and_anchors[0]
            .anchored_event_ids
            .clear();
        satisfy.settlement_basis = Some(watermark_anchored_violation_basis(&satisfy));
        assert_eq!(
            Verdict::try_from(verdict(satisfy)),
            Err(VerdictError::SettlementBasisMismatch)
        );
    }

    #[test]
    fn anchor_silence_names_the_residual_presence_case() {
        let mut envelope = assurance();
        envelope.source_tiers_and_anchors[0]
            .anchored_event_ids
            .clear();
        envelope.settlement_basis = None;
        envelope.source_tier_gap = Some(raw_model::SourceTierGap {
            contributing_sources: Vec::new(),
            detail: "the conclusion rests on no anchor-tier Event".to_owned(),
            unanchored_sources: vec![source()],
        });
        let mut residual = verdict(envelope);
        residual.outcome = raw_model::VerdictOutcome::Inconclusive as i32;
        residual.inconclusive_reason = Some(raw_model::InconclusiveReason::AnchorSilence as i32);
        let settled = Verdict::try_from(residual).expect("ADR-0033 residual is nameable");
        assert_eq!(
            settled.inconclusive_reason(),
            Some(InconclusiveReason::AnchorSilence)
        );
        assert!(settled.source_tier_gap().is_some());
    }

    #[test]
    fn authoritative_verdict_validates_complete_nested_envelope() {
        Verdict::try_from(verdict(assurance())).expect("complete envelope is valid");
    }

    #[test]
    fn authoritative_satisfy_rejects_every_open_evidence_condition() {
        type OpenEvidence = fn(&mut raw_model::AssuranceEnvelope);
        let cases: [(&str, OpenEvidence); 5] = [
            ("assurance.source_health", |value| {
                value.source_health[0].healthy = false;
                value.source_health[0].detail = "heartbeat expired".to_owned();
            }),
            ("assurance.gaps", |value| {
                value.gaps.push(raw_model::SourceGap {
                    source: Some(source()),
                    sequence_epoch: 1,
                    first_missing: 2,
                    last_missing: 2,
                    effect: "can hide a violation".to_owned(),
                });
            }),
            ("assurance.unresolved_causal_edges", |value| {
                value
                    .unresolved_causal_edges
                    .push(raw_model::UnresolvedCausalEdge {
                        event_id: "later".to_owned(),
                        missing_predecessor_id: "earlier".to_owned(),
                    });
            }),
            ("assurance.extraction_facts", |value| {
                value.extraction_facts[0].available = false;
                value.extraction_facts[0].detail = "payload was opaque".to_owned();
            }),
            ("assurance.uncertainty", |value| {
                value.uncertainty.push(raw_model::Uncertainty {
                    reason: raw_model::InconclusiveReason::PrefixUncertainty as i32,
                    detail: "required evidence is missing".to_owned(),
                });
            }),
        ];
        for (field, open) in cases {
            let mut envelope = assurance();
            open(&mut envelope);
            assert_eq!(
                Verdict::try_from(verdict(envelope)),
                Err(VerdictError::AuthoritativeEvidenceIncomplete(field)),
                "open {field} must not become an authoritative pass"
            );
        }
    }

    #[test]
    fn reviewer_combined_contradictory_satisfy_fixture_is_rejected() {
        let mut envelope = assurance();
        envelope.source_health[0].healthy = false;
        envelope.source_health[0].detail = "heartbeat expired".to_owned();
        envelope.gaps.push(raw_model::SourceGap {
            source: Some(source()),
            sequence_epoch: 1,
            first_missing: 2,
            last_missing: 2,
            effect: "can hide a violation".to_owned(),
        });
        envelope
            .unresolved_causal_edges
            .push(raw_model::UnresolvedCausalEdge {
                event_id: "later".to_owned(),
                missing_predecessor_id: "earlier".to_owned(),
            });
        envelope.extraction_facts[0].available = false;
        envelope.extraction_facts[0].detail = "payload was opaque".to_owned();
        envelope.uncertainty.push(raw_model::Uncertainty {
            reason: raw_model::InconclusiveReason::PrefixUncertainty as i32,
            detail: "required evidence is missing".to_owned(),
        });

        assert_eq!(
            Verdict::try_from(verdict(envelope)),
            Err(VerdictError::AuthoritativeEvidenceIncomplete(
                "assurance.source_health"
            ))
        );
    }

    /// The order the assurance checks run in is itself a behavioural claim.
    ///
    /// Every row breaks two conditions at once and names the error the envelope must
    /// report: the one belonging to the check that runs first. Callers route on the
    /// variant, so which defect a multi-defect envelope surfaces is observable, and
    /// nothing but this test forbids a refactor from quietly moving a check across a
    /// phase boundary. The rows are chosen to straddle each boundary in turn --
    /// authority/tier settlement, the required-source set, the subset sweep, the
    /// authoritative-coverage sweep, the source-tier gap, the settlement watermarks
    /// and the outcome basis -- so that a reshuffle fails here rather than in
    /// production.
    #[test]
    #[allow(clippy::too_many_lines)] // One row per boundary; the whole precedence ladder reads together.
    fn assurance_validation_reports_the_first_failing_condition() {
        type Contradiction = fn(&mut raw_model::Verdict);
        let cases: [(&str, Contradiction, VerdictError); 8] = [
            (
                "the authority/tier-settlement match precedes every collection check",
                |value| {
                    value.authority = raw_model::VerdictAuthority::Provisional as i32;
                    value
                        .assurance
                        .as_mut()
                        .expect("fixture envelope")
                        .required_sources
                        .clear();
                },
                VerdictError::ProvisionalTierMismatch,
            ),
            (
                "the same match on its authoritative arm precedes the envelope scalars",
                |value| {
                    let envelope = value.assurance.as_mut().expect("fixture envelope");
                    envelope.hot_path = raw_model::TierSettlement::Pending as i32;
                    envelope.specification_id = String::new();
                },
                VerdictError::AuthoritativeTierUnsettled,
            ),
            (
                "the required-source set is established before anything is compared to it",
                |value| {
                    let envelope = value.assurance.as_mut().expect("fixture envelope");
                    envelope.required_sources.clear();
                    envelope.observed_sources.clear();
                },
                VerdictError::MissingField("assurance.required_sources"),
            ),
            (
                "the subset sweep precedes the authoritative-coverage sweep",
                |value| {
                    let envelope = value.assurance.as_mut().expect("fixture envelope");
                    envelope.observed_sources = vec![source_named("ledger")];
                    envelope.filter_epochs.clear();
                },
                VerdictError::AssuranceSourceMismatch("assurance.observed_sources"),
            ),
            (
                "the authoritative-coverage sweep reports observed sources before capabilities",
                |value| {
                    let envelope = value.assurance.as_mut().expect("fixture envelope");
                    envelope.observed_sources.clear();
                    envelope.capabilities.clear();
                },
                VerdictError::AuthoritativeCoverageIncomplete("assurance.observed_sources"),
            ),
            (
                "the source-tier gap precedes the settlement watermarks",
                |value| {
                    let envelope = value.assurance.as_mut().expect("fixture envelope");
                    envelope.source_tiers_and_anchors[0].tier =
                        raw_model::SourceTier::Contributing as i32;
                    envelope.source_tiers_and_anchors[0]
                        .anchored_event_ids
                        .clear();
                    envelope.hot_settlement_watermarks.clear();
                },
                VerdictError::SourceTierGapMismatch,
            ),
            (
                "the settlement watermarks precede the outcome basis",
                |value| {
                    let envelope = value.assurance.as_mut().expect("fixture envelope");
                    envelope.hot_settlement_watermarks.clear();
                    envelope.gaps.push(raw_model::SourceGap {
                        source: Some(source()),
                        sequence_epoch: 1,
                        first_missing: 2,
                        last_missing: 2,
                        effect: "can hide a violation".to_owned(),
                    });
                },
                VerdictError::SettlementWatermarkMismatch("assurance.hot_settlement_watermarks"),
            ),
            (
                "the decisive-basis header precedes the per-witness checks",
                |value| {
                    let envelope = value.assurance.as_mut().expect("fixture envelope");
                    let mut basis = decisive_violation_basis(envelope);
                    if let Some(raw_model::settlement_basis::Basis::DecisiveViolation(decisive)) =
                        basis.basis.as_mut()
                    {
                        decisive.invariance_proof_rule_id = String::new();
                        decisive.witness_events[0].source = Some(source_named("ledger"));
                    }
                    envelope.settlement_basis = Some(basis);
                    value.outcome = raw_model::VerdictOutcome::Violate as i32;
                },
                VerdictError::MissingScalar("assurance.settlement_basis.invariance_proof_rule_id"),
            ),
        ];

        for (pins, contradict, expected) in cases {
            let mut value = verdict(assurance());
            contradict(&mut value);
            assert_eq!(
                Verdict::try_from(value),
                Err(expected),
                "precedence changed: {pins}"
            );
        }
    }

    #[test]
    fn contributing_only_evidence_requires_typed_gap_and_cannot_authoritatively_satisfy() {
        let mut envelope = assurance();
        envelope.source_tiers_and_anchors[0].tier = raw_model::SourceTier::Contributing as i32;
        envelope.source_tiers_and_anchors[0]
            .anchored_event_ids
            .clear();
        assert_eq!(
            Verdict::try_from(verdict(envelope.clone())),
            Err(VerdictError::SourceTierGapMismatch)
        );

        envelope.source_tier_gap = Some(raw_model::SourceTierGap {
            contributing_sources: vec![source()],
            detail: "no unsuppressable source is deployed".to_owned(),
            unanchored_sources: vec![source()],
        });
        assert_eq!(
            Verdict::try_from(verdict(envelope)),
            Err(VerdictError::MissingAnchorEvidence)
        );
    }

    #[test]
    #[allow(clippy::too_many_lines)] // Every source-tier coverage boundary stays auditable together.
    fn source_tier_gap_exactly_names_contributing_and_unanchored_sources() {
        let gateway = source();
        let ledger = source_named("ledger");
        let required = BTreeSet::from([owned_source_key(&gateway), owned_source_key(&ledger)]);
        let record = |source: raw_model::SourceIdentity,
                      tier: raw_model::SourceTier,
                      anchored: bool| raw_model::SourceTierRecord {
            source: Some(source),
            tier: tier as i32,
            anchored_event_ids: anchored.then(|| "event".to_owned()).into_iter().collect(),
        };
        let gap = |contributing_sources: Vec<raw_model::SourceIdentity>,
                   unanchored_sources: Vec<raw_model::SourceIdentity>| {
            raw_model::SourceTierGap {
                contributing_sources,
                detail: "required source-tier evidence is incomplete".to_owned(),
                unanchored_sources,
            }
        };
        let validate = |records: Vec<raw_model::SourceTierRecord>,
                        gap: Option<raw_model::SourceTierGap>| {
            let evidence = validate_tier_records(&records).expect("valid tier records");
            AssuranceValidator::validate_source_tier_gap(
                gap.as_ref(),
                &required,
                &evidence,
                &BTreeSet::new(),
            )
        };

        let full_with_anchor = vec![
            record(gateway.clone(), raw_model::SourceTier::Anchor, true),
            record(ledger.clone(), raw_model::SourceTier::Contributing, false),
        ];
        assert_eq!(validate(full_with_anchor.clone(), None), Ok(()));
        assert_eq!(
            validate(
                full_with_anchor,
                Some(gap(vec![ledger.clone()], vec![ledger.clone()]))
            ),
            Err(VerdictError::SourceTierGapMismatch)
        );

        let full_contributing = vec![
            record(gateway.clone(), raw_model::SourceTier::Contributing, false),
            record(ledger.clone(), raw_model::SourceTier::Contributing, false),
        ];
        assert_eq!(
            validate(full_contributing.clone(), None),
            Err(VerdictError::SourceTierGapMismatch)
        );
        assert_eq!(
            validate(
                full_contributing.clone(),
                Some(gap(
                    vec![ledger.clone(), gateway.clone()],
                    vec![gateway.clone(), ledger.clone()],
                ))
            ),
            Ok(())
        );
        assert_eq!(
            validate(
                full_contributing,
                Some(gap(
                    vec![gateway.clone()],
                    vec![gateway.clone(), ledger.clone()],
                ))
            ),
            Err(VerdictError::SourceTierGapMismatch)
        );

        let partial_anchored = vec![record(gateway.clone(), raw_model::SourceTier::Anchor, true)];
        assert_eq!(
            validate(partial_anchored.clone(), None),
            Err(VerdictError::SourceTierGapMismatch)
        );
        assert_eq!(
            validate(
                partial_anchored.clone(),
                Some(gap(Vec::new(), vec![ledger.clone()]))
            ),
            Ok(())
        );
        assert_eq!(
            validate(
                partial_anchored,
                Some(gap(Vec::new(), vec![gateway.clone()]))
            ),
            Err(VerdictError::SourceTierGapMismatch)
        );

        let partial_contributing = vec![record(
            gateway.clone(),
            raw_model::SourceTier::Contributing,
            false,
        )];
        assert_eq!(
            validate(partial_contributing.clone(), None),
            Err(VerdictError::SourceTierGapMismatch)
        );
        assert_eq!(
            validate(
                partial_contributing.clone(),
                Some(gap(
                    vec![gateway.clone()],
                    vec![ledger.clone(), gateway.clone()],
                ))
            ),
            Ok(())
        );
        assert_eq!(
            validate(
                partial_contributing,
                Some(gap(vec![gateway.clone()], vec![gateway.clone()]))
            ),
            Err(VerdictError::SourceTierGapMismatch)
        );

        let full_unanchored = vec![
            record(gateway.clone(), raw_model::SourceTier::Anchor, false),
            record(ledger.clone(), raw_model::SourceTier::Anchor, false),
        ];
        assert_eq!(
            validate(full_unanchored.clone(), None),
            Err(VerdictError::SourceTierGapMismatch)
        );
        assert_eq!(
            validate(
                full_unanchored,
                Some(gap(Vec::new(), vec![gateway, ledger]))
            ),
            Ok(())
        );
    }

    #[test]
    fn quiet_anchor_sources_never_downgrade_a_verdict_that_keeps_one_genuine_anchor() {
        let gateway = source();
        let permission = source_named("permission");
        let ledger = source_named("ledger");
        let required = BTreeSet::from([
            owned_source_key(&gateway),
            owned_source_key(&permission),
            owned_source_key(&ledger),
        ]);
        let record = |source: raw_model::SourceIdentity,
                      tier: raw_model::SourceTier,
                      anchored: bool| raw_model::SourceTierRecord {
            source: Some(source),
            tier: tier as i32,
            anchored_event_ids: anchored.then(|| "event".to_owned()).into_iter().collect(),
        };
        let gap = |contributing_sources: Vec<raw_model::SourceIdentity>,
                   unanchored_sources: Vec<raw_model::SourceIdentity>| {
            raw_model::SourceTierGap {
                contributing_sources,
                detail: "required source-tier evidence is incomplete".to_owned(),
                unanchored_sources,
            }
        };
        let validate = |records: Vec<raw_model::SourceTierRecord>,
                        gap: Option<raw_model::SourceTierGap>| {
            let evidence = validate_tier_records(&records).expect("valid tier records");
            AssuranceValidator::validate_source_tier_gap(
                gap.as_ref(),
                &required,
                &evidence,
                &BTreeSet::new(),
            )
        };

        // Two quiet anchor-tier sources alongside one that genuinely anchored: the downgrade
        // is owed collectively, so no gap is required (ADR-0022).
        let quiet_anchors = vec![
            record(gateway, raw_model::SourceTier::Anchor, true),
            record(permission.clone(), raw_model::SourceTier::Anchor, false),
            record(ledger.clone(), raw_model::SourceTier::Anchor, false),
        ];
        assert_eq!(validate(quiet_anchors.clone(), None), Ok(()));
        // The collective downgrade is not voluntarily declarable for the quiet sources.
        assert_eq!(
            validate(
                quiet_anchors,
                Some(gap(Vec::new(), vec![permission, ledger]))
            ),
            Err(VerdictError::SourceTierGapMismatch)
        );
    }

    #[test]
    fn unproven_anchor_source_is_named_unanchored_and_never_contributing() {
        let gateway = source();
        let permission = source_named("permission");
        let agent = source_named("agent");
        let required = BTreeSet::from([
            owned_source_key(&gateway),
            owned_source_key(&permission),
            owned_source_key(&agent),
        ]);
        let record = |source: raw_model::SourceIdentity,
                      tier: raw_model::SourceTier,
                      anchored: bool| raw_model::SourceTierRecord {
            source: Some(source),
            tier: tier as i32,
            anchored_event_ids: anchored.then(|| "event".to_owned()).into_iter().collect(),
        };
        let gap = |contributing_sources: Vec<raw_model::SourceIdentity>,
                   unanchored_sources: Vec<raw_model::SourceIdentity>| {
            raw_model::SourceTierGap {
                contributing_sources,
                detail: "required source-tier evidence is incomplete".to_owned(),
                unanchored_sources,
            }
        };
        let validate = |records: Vec<raw_model::SourceTierRecord>,
                        gap: Option<raw_model::SourceTierGap>| {
            let evidence = validate_tier_records(&records).expect("valid tier records");
            AssuranceValidator::validate_source_tier_gap(
                gap.as_ref(),
                &required,
                &evidence,
                &BTreeSet::new(),
            )
        };

        // No required source carries valid anchor evidence, so a gap is owed.
        let unproven_anchor = vec![
            record(gateway.clone(), raw_model::SourceTier::Contributing, false),
            record(permission.clone(), raw_model::SourceTier::Anchor, false),
            record(agent.clone(), raw_model::SourceTier::Contributing, false),
        ];
        assert_eq!(
            validate(unproven_anchor.clone(), None),
            Err(VerdictError::SourceTierGapMismatch)
        );
        // The quiet anchor-tier source lands in exactly one of the two sets.
        assert_eq!(
            validate(
                unproven_anchor.clone(),
                Some(gap(
                    vec![gateway.clone(), agent.clone()],
                    vec![gateway.clone(), permission.clone(), agent.clone()],
                ))
            ),
            Ok(())
        );
        // An anchor-tier source is never laundered into the declared contributing set.
        assert_eq!(
            validate(
                unproven_anchor.clone(),
                Some(gap(
                    vec![gateway.clone(), permission.clone(), agent.clone()],
                    vec![gateway.clone(), permission, agent.clone()],
                ))
            ),
            Err(VerdictError::SourceTierGapMismatch)
        );
        // Nor may it be omitted from the unanchored set.
        assert_eq!(
            validate(
                unproven_anchor,
                Some(gap(
                    vec![gateway.clone(), agent.clone()],
                    vec![gateway, agent],
                ))
            ),
            Err(VerdictError::SourceTierGapMismatch)
        );
    }

    #[test]
    fn partial_contributing_coverage_requires_gap_at_verdict_boundary() {
        let ledger = source_named("ledger");
        let mut envelope = assurance();
        envelope.required_sources.push(ledger.clone());
        envelope.source_tiers_and_anchors[0].tier = raw_model::SourceTier::Contributing as i32;
        envelope.source_tiers_and_anchors[0]
            .anchored_event_ids
            .clear();
        envelope.cold_path = raw_model::TierSettlement::Pending as i32;
        envelope.settlement_basis = None;

        let mut provisional = verdict(envelope.clone());
        provisional.authority = raw_model::VerdictAuthority::Provisional as i32;
        assert_eq!(
            Verdict::try_from(provisional),
            Err(VerdictError::SourceTierGapMismatch)
        );

        envelope.source_tier_gap = Some(raw_model::SourceTierGap {
            contributing_sources: vec![source()],
            detail: "ledger tier evidence is missing".to_owned(),
            unanchored_sources: vec![ledger, source()],
        });
        let mut provisional = verdict(envelope.clone());
        provisional.authority = raw_model::VerdictAuthority::Provisional as i32;
        let canonical =
            Verdict::try_from(provisional).expect("an exact tier gap remains provisional");
        let gap = envelope
            .source_tier_gap
            .as_mut()
            .expect("fixture has a source-tier gap");
        gap.contributing_sources.reverse();
        gap.unanchored_sources.reverse();
        let mut permuted = verdict(envelope.clone());
        permuted.authority = raw_model::VerdictAuthority::Provisional as i32;
        let permuted = Verdict::try_from(permuted).expect("permuted tier gap remains valid");
        assert_eq!(canonical, permuted);
        assert_eq!(canonical.encode_to_vec(), permuted.encode_to_vec());

        let mut inconclusive = verdict(envelope.clone());
        inconclusive.authority = raw_model::VerdictAuthority::Provisional as i32;
        inconclusive.outcome = raw_model::VerdictOutcome::Inconclusive as i32;
        inconclusive.inconclusive_reason =
            Some(raw_model::InconclusiveReason::PrefixUncertainty as i32);
        Verdict::try_from(inconclusive).expect("an exact tier gap remains inconclusive");

        envelope.cold_path = raw_model::TierSettlement::NotRequired as i32;
        envelope.settlement_basis = Some(raw_model::SettlementBasis {
            basis: Some(raw_model::settlement_basis::Basis::CompleteEvidence(
                raw_model::CompleteEvidenceBasis {},
            )),
        });
        assert_eq!(
            Verdict::try_from(verdict(envelope)),
            Err(VerdictError::AuthoritativeCoverageIncomplete(
                "assurance.observed_sources"
            ))
        );
    }

    #[test]
    fn authoritative_violation_accepts_closed_decisive_witness_with_explicit_limitations() {
        let mut envelope = assurance();
        envelope.gaps.push(raw_model::SourceGap {
            source: Some(source()),
            sequence_epoch: 1,
            first_missing: 2,
            last_missing: 2,
            effect: "irrelevant to the existence witness".to_owned(),
        });
        envelope
            .unresolved_causal_edges
            .push(raw_model::UnresolvedCausalEdge {
                event_id: "unrelated".to_owned(),
                missing_predecessor_id: "unknown".to_owned(),
            });
        envelope.extraction_facts.push(raw_model::ExtractionFact {
            fact_id: "unrelated.memo".to_owned(),
            available: false,
            detail: "opaque but not used by the witness".to_owned(),
        });
        envelope.uncertainty.push(raw_model::Uncertainty {
            reason: raw_model::InconclusiveReason::PrefixUncertainty as i32,
            detail: "the named proof establishes outcome invariance".to_owned(),
        });
        envelope.settlement_basis = Some(decisive_violation_basis(&envelope));
        let mut violation = verdict(envelope);
        violation.outcome = raw_model::VerdictOutcome::Violate as i32;

        Verdict::try_from(violation)
            .expect("a closed anchored witness may prove a violation despite listed limitations");
    }

    #[test]
    fn authoritative_violation_requires_decisive_basis_not_only_complete_coverage() {
        let mut violation = verdict(assurance());
        violation.outcome = raw_model::VerdictOutcome::Violate as i32;
        assert_eq!(
            Verdict::try_from(violation),
            Err(VerdictError::SettlementBasisMismatch)
        );
    }

    #[test]
    fn decisive_violation_rejects_unlisted_or_witness_intersecting_limitations() {
        let mut unlisted = assurance();
        unlisted.gaps.push(raw_model::SourceGap {
            source: Some(source()),
            sequence_epoch: 1,
            first_missing: 2,
            last_missing: 2,
            effect: "outcome invariant".to_owned(),
        });
        let mut basis = decisive_violation_basis(&unlisted);
        let Some(raw_model::settlement_basis::Basis::DecisiveViolation(basis)) =
            basis.basis.as_mut()
        else {
            panic!("decisive fixture must contain decisive basis");
        };
        basis
            .outcome_invariant_limitations
            .as_mut()
            .expect("fixture limitations")
            .gaps
            .clear();
        unlisted.settlement_basis = Some(raw_model::SettlementBasis {
            basis: Some(raw_model::settlement_basis::Basis::DecisiveViolation(
                basis.clone(),
            )),
        });
        let mut violation = verdict(unlisted);
        violation.outcome = raw_model::VerdictOutcome::Violate as i32;
        assert_eq!(
            Verdict::try_from(violation),
            Err(VerdictError::InvariantLimitationsMismatch("gaps"))
        );

        let mut intersecting = assurance();
        intersecting.gaps.push(raw_model::SourceGap {
            source: Some(source()),
            sequence_epoch: 1,
            first_missing: 4,
            last_missing: 4,
            effect: "covers the alleged witness".to_owned(),
        });
        intersecting.settlement_basis = Some(decisive_violation_basis(&intersecting));
        let mut violation = verdict(intersecting);
        violation.outcome = raw_model::VerdictOutcome::Violate as i32;
        assert_eq!(
            Verdict::try_from(violation),
            Err(VerdictError::InvalidDecisiveWitness("sequence_gap"))
        );
    }

    #[test]
    fn provisional_and_inconclusive_verdicts_preserve_explicit_gaps() {
        let mut provisional_envelope = assurance();
        provisional_envelope.source_tiers_and_anchors[0].tier =
            raw_model::SourceTier::Contributing as i32;
        provisional_envelope.source_tiers_and_anchors[0]
            .anchored_event_ids
            .clear();
        provisional_envelope.source_tier_gap = Some(raw_model::SourceTierGap {
            contributing_sources: vec![source()],
            detail: "only application-owned evidence is available".to_owned(),
            unanchored_sources: vec![source()],
        });
        provisional_envelope.cold_path = raw_model::TierSettlement::Pending as i32;
        provisional_envelope.settlement_basis = None;
        let mut provisional = verdict(provisional_envelope);
        provisional.authority = raw_model::VerdictAuthority::Provisional as i32;
        Verdict::try_from(provisional).expect("typed source-tier gaps remain provisional");

        let mut inconclusive_envelope = assurance();
        inconclusive_envelope.gaps.push(raw_model::SourceGap {
            source: Some(source()),
            sequence_epoch: 1,
            first_missing: 2,
            last_missing: 2,
            effect: "may contain required evidence".to_owned(),
        });
        inconclusive_envelope
            .uncertainty
            .push(raw_model::Uncertainty {
                reason: raw_model::InconclusiveReason::PrefixUncertainty as i32,
                detail: "the gap prevents settlement".to_owned(),
            });
        inconclusive_envelope.settlement_basis = None;
        let mut inconclusive = verdict(inconclusive_envelope);
        inconclusive.outcome = raw_model::VerdictOutcome::Inconclusive as i32;
        inconclusive.inconclusive_reason =
            Some(raw_model::InconclusiveReason::PrefixUncertainty as i32);
        Verdict::try_from(inconclusive).expect("prefix uncertainty remains inconclusive");
    }

    #[test]
    fn verdict_inconclusive_reason_taxonomy_includes_anchor_silence() {
        for reason in [
            raw_model::InconclusiveReason::PrefixUncertainty,
            raw_model::InconclusiveReason::ObservationalUncertainty,
            raw_model::InconclusiveReason::EpistemicUncertainty,
            raw_model::InconclusiveReason::AnchorSilence,
        ] {
            let mut envelope = assurance();
            envelope.settlement_basis = None;
            let mut inconclusive = verdict(envelope);
            inconclusive.outcome = raw_model::VerdictOutcome::Inconclusive as i32;
            inconclusive.inconclusive_reason = Some(reason as i32);
            Verdict::try_from(inconclusive).expect("ADR-0018 reason remains valid");
        }

        assert!(raw_model::InconclusiveReason::try_from(4).is_err());
        assert!(
            raw_model::InconclusiveReason::from_str_name("INCONCLUSIVE_REASON_EVIDENCE_GAP")
                .is_none()
        );

        let mut envelope = assurance();
        envelope.settlement_basis = None;
        let mut reserved = verdict(envelope);
        reserved.outcome = raw_model::VerdictOutcome::Inconclusive as i32;
        reserved.inconclusive_reason = Some(4);
        assert_eq!(
            Verdict::try_from(reserved),
            Err(VerdictError::VerdictReasonMismatch)
        );
    }

    #[test]
    fn authoritative_evidence_must_reach_settlement_and_name_filter_epoch() {
        let mut behind = assurance();
        behind.watermarks[0].complete_through = 3;
        assert_eq!(
            Verdict::try_from(verdict(behind)),
            Err(VerdictError::SettlementEvidenceIncomplete(
                "assurance.hot_settlement_watermarks"
            ))
        );

        let mut missing_filter_epoch = assurance();
        missing_filter_epoch.filter_epochs[0].filter_epoch = None;
        assert_eq!(
            Verdict::try_from(verdict(missing_filter_epoch)),
            Err(VerdictError::MissingScalar(
                "assurance.filter_epoch.filter_epoch"
            ))
        );
    }

    #[test]
    fn authoritative_verdict_requires_complete_required_source_coverage() {
        type ClearEvidence = fn(&mut raw_model::AssuranceEnvelope);
        let cases: [(&str, ClearEvidence); 6] = [
            ("assurance.observed_sources", |value| {
                value.observed_sources.clear();
            }),
            ("assurance.filter_epochs", |value| {
                value.filter_epochs.clear();
            }),
            ("assurance.watermarks", |value| value.watermarks.clear()),
            ("assurance.source_health", |value| {
                value.source_health.clear();
            }),
            ("assurance.capabilities", |value| value.capabilities.clear()),
            ("assurance.source_tiers_and_anchors", |value| {
                value.source_tiers_and_anchors.clear();
            }),
        ];
        for (field, clear) in cases {
            let mut partial = assurance();
            clear(&mut partial);
            assert_eq!(
                Verdict::try_from(verdict(partial)),
                Err(VerdictError::AuthoritativeCoverageIncomplete(field)),
                "missing {field} must not validate authoritatively"
            );
        }
    }

    #[test]
    fn provisional_verdict_allows_partial_coverage_but_requires_hot_settlement() {
        let mut partial = assurance();
        partial.observed_sources.clear();
        partial.filter_epochs.clear();
        partial.watermarks.clear();
        partial.source_health.clear();
        partial.capabilities.clear();
        partial.source_tiers_and_anchors.clear();
        partial.source_tier_gap = Some(raw_model::SourceTierGap {
            contributing_sources: Vec::new(),
            detail: "source tier evidence has not arrived".to_owned(),
            unanchored_sources: vec![source()],
        });
        partial.cold_path = raw_model::TierSettlement::Pending as i32;
        partial.settlement_basis = None;
        let mut provisional = verdict(partial);
        provisional.authority = raw_model::VerdictAuthority::Provisional as i32;
        Verdict::try_from(provisional).expect("partial evidence remains explicitly provisional");

        let mut invalid = verdict(assurance());
        invalid.authority = raw_model::VerdictAuthority::Provisional as i32;
        assert_eq!(
            Verdict::try_from(invalid),
            Err(VerdictError::ProvisionalTierMismatch)
        );
    }

    /// An Anchor Source that anchored *something* still cannot anchor *this* witness.
    ///
    /// The Source keeps its Anchor tier and a non-empty anchor list, so no Source Tier Gap
    /// is owed and every per-witness gate passes -- settlement, health, sequence gaps,
    /// causal edges. What fails is the last question ADR-0022 asks: is the witness event
    /// itself among the events this Source anchored. Nothing else in the module isolates
    /// that, because reaching it through `Verdict::try_from` means getting eight earlier
    /// conditions right at the same time.
    #[test]
    fn authoritative_violation_without_an_anchored_witness_is_rejected() {
        let mut envelope = assurance();
        envelope.source_tiers_and_anchors[0].anchored_event_ids = vec!["other".to_owned()];
        let basis = decisive_violation_basis(&envelope);
        envelope.settlement_basis = Some(basis);

        assert_eq!(
            AssuranceValidator::check(
                &envelope,
                raw_model::VerdictOutcome::Violate,
                raw_model::VerdictAuthority::Authoritative,
            ),
            Err(VerdictError::MissingAnchorEvidence)
        );
    }

    /// An open Source Gap denies an authoritative satisfaction.
    ///
    /// The same claim as the `assurance.gaps` row of
    /// `authoritative_satisfy_rejects_every_open_evidence_condition`, asserted at the
    /// envelope seam instead of through a whole Verdict. It is here because ADR-0017's
    /// rule deserves to be stated where the rule lives, not because it covers new ground.
    #[test]
    fn authoritative_satisfaction_with_open_gaps_is_rejected() {
        let mut envelope = assurance();
        envelope.gaps.push(raw_model::SourceGap {
            source: Some(source()),
            sequence_epoch: 1,
            first_missing: 2,
            last_missing: 2,
            effect: "can hide a violation".to_owned(),
        });

        assert_eq!(
            AssuranceValidator::check(
                &envelope,
                raw_model::VerdictOutcome::Satisfy,
                raw_model::VerdictAuthority::Authoritative,
            ),
            Err(VerdictError::AuthoritativeEvidenceIncomplete(
                "assurance.gaps"
            ))
        );
    }

    /// Provisional requires a settled hot path and either a pending cold path or a
    /// pending activation barrier.
    ///
    /// The same fixture with `NOT_DECLARED` still rejects: a cold path of `NotRequired`
    /// is what an authoritative Verdict claims, and reusing it under provisional
    /// authority with no pending barrier would let a Verdict be provisional about
    /// nothing. The same fixture with `PENDING` and `cold = NOT_REQUIRED` validates.
    #[test]
    fn provisional_authority_requires_a_pending_cold_path() {
        assert_eq!(
            AssuranceValidator::check(
                &assurance(),
                raw_model::VerdictOutcome::Satisfy,
                raw_model::VerdictAuthority::Provisional,
            ),
            Err(VerdictError::ProvisionalTierMismatch)
        );

        let mut pending_barrier = assurance();
        pending_barrier.activation_barrier = raw_model::ActivationBarrierStatus::Pending as i32;
        pending_barrier.target_filter_epoch = Some(9);
        pending_barrier.settlement_basis = None;
        assert_eq!(
            AssuranceValidator::check(
                &pending_barrier,
                raw_model::VerdictOutcome::Satisfy,
                raw_model::VerdictAuthority::Provisional,
            ),
            Ok(())
        );
    }

    /// The shape ADR-0021 does allow: settled hot path, pending cold path, no basis named.
    ///
    /// The positive counterpart to the previous test. A provisional Verdict names no
    /// Settlement Basis because it is not claiming settlement -- that is the whole
    /// difference between provisional and authoritative -- so this must validate.
    #[test]
    fn provisional_verdict_with_settled_hot_and_pending_cold_path_validates() {
        let mut envelope = assurance();
        envelope.cold_path = raw_model::TierSettlement::Pending as i32;
        envelope.settlement_basis = None;

        assert_eq!(
            AssuranceValidator::check(
                &envelope,
                raw_model::VerdictOutcome::Satisfy,
                raw_model::VerdictAuthority::Provisional,
            ),
            Ok(())
        );
    }

    /// A witness the Watermark has not reached yet cannot be decisive.
    ///
    /// The witness sits at sequence number 5 while the Source's Watermark completes through
    /// 4, so the event is not settled: more evidence for that Source can still arrive at or
    /// before it. Everything else about the basis is well-formed, which is what makes the
    /// settlement rule the one under test.
    #[test]
    fn decisive_witness_behind_the_settlement_watermark_is_rejected() {
        let mut envelope = assurance();
        let mut basis = decisive_violation_basis(&envelope);
        if let Some(raw_model::settlement_basis::Basis::DecisiveViolation(decisive)) =
            basis.basis.as_mut()
        {
            decisive.witness_events[0].sequence_number = Some(5);
        }
        envelope.settlement_basis = Some(basis);

        assert_eq!(
            AssuranceValidator::check(
                &envelope,
                raw_model::VerdictOutcome::Violate,
                raw_model::VerdictAuthority::Authoritative,
            ),
            Err(VerdictError::InvalidDecisiveWitness("settlement"))
        );
    }

    #[test]
    fn verdict_accessors_read_back_the_validated_envelope() {
        let settled =
            Verdict::try_from(verdict(assurance())).expect("baseline verdict is constructible");
        assert_eq!(settled.id(), "verdict");
        assert_eq!(settled.outcome(), VerdictOutcome::Satisfy);
        assert_eq!(settled.inconclusive_reason(), None);
        assert_eq!(settled.authority(), VerdictAuthority::Authoritative);
        assert_eq!(settled.specification_id(), "spec");
        assert_eq!(settled.specification_version(), "v1");
        assert_eq!(settled.required_sources(), settled.observed_sources());
        assert!(settled.gaps().is_empty());
        assert!(settled.uncertainty().is_empty());
        assert!(settled.unresolved_causal_edges().is_empty());
        assert!(settled.unhealthy_sources().is_empty());
        assert_eq!(settled.hot_path(), TierSettlement::Settled);
        assert_eq!(settled.cold_path(), TierSettlement::NotRequired);
        assert_eq!(
            settled.settlement_basis(),
            Some(SettlementBasisKind::CompleteEvidence)
        );
        assert_eq!(settled.source_tier_gap(), None);
        assert_eq!(
            settled
                .anchored_event_ids()
                .into_values()
                .flatten()
                .map(|id| id.to_string())
                .collect::<Vec<_>>(),
            vec!["event".to_owned()]
        );
        assert!(settled.cold_settlement_watermarks().is_empty());
        assert_eq!(settled.watermarks(), settled.hot_settlement_watermarks());
        assert!(settled
            .capabilities()
            .into_values()
            .all(|capabilities| capabilities.contains(SourceCapability::CausalEdges)));

        let mut inconclusive = verdict(assurance());
        inconclusive.outcome = raw_model::VerdictOutcome::Inconclusive as i32;
        inconclusive.inconclusive_reason =
            Some(raw_model::InconclusiveReason::ObservationalUncertainty as i32);
        inconclusive
            .assurance
            .as_mut()
            .expect("assurance is present")
            .settlement_basis = None;
        let inconclusive =
            Verdict::try_from(inconclusive).expect("inconclusive verdict is constructible");
        assert_eq!(inconclusive.outcome(), VerdictOutcome::Inconclusive);
        assert_eq!(
            inconclusive.inconclusive_reason(),
            Some(InconclusiveReason::ObservationalUncertainty)
        );
        assert_eq!(inconclusive.settlement_basis(), None);
        assert!(inconclusive.decisive_witness_event_ids().is_empty());
        assert!(inconclusive.outcome_invariant_uncertainty().is_empty());
    }

    #[test]
    fn verdict_rejects_evidence_from_sources_outside_the_required_set() {
        let mut foreign = source();
        foreign.source_id = "other".to_owned();
        let mut envelope = assurance();
        envelope.observed_sources[0] = foreign;
        assert_eq!(
            Verdict::try_from(verdict(envelope)),
            Err(VerdictError::AssuranceSourceMismatch(
                "assurance.observed_sources"
            ))
        );
    }

    #[test]
    fn authoritative_verdict_requires_settlement_watermarks() {
        let mut envelope = assurance();
        envelope.hot_settlement_watermarks.clear();
        assert_eq!(
            Verdict::try_from(verdict(envelope)),
            Err(VerdictError::SettlementWatermarkMismatch(
                "assurance.hot_settlement_watermarks"
            ))
        );

        let mut cold = assurance();
        cold.cold_path = raw_model::TierSettlement::Settled as i32;
        assert_eq!(
            Verdict::try_from(verdict(cold)),
            Err(VerdictError::SettlementWatermarkMismatch(
                "assurance.cold_settlement_watermarks"
            ))
        );
    }

    #[test]
    fn verdict_rejects_unknown_nested_capability() {
        let mut assurance = assurance();
        assurance.capabilities[0].capabilities[0] = 99;
        assert_eq!(
            Verdict::try_from(verdict(assurance)),
            Err(VerdictError::UnknownEnum {
                field: "assurance.source_capability",
                value: 99,
            })
        );
    }

    #[test]
    fn verdict_rejects_missing_nested_source_and_time() {
        let mut missing_source = assurance();
        missing_source.filter_epochs[0].source = None;
        assert_eq!(
            Verdict::try_from(verdict(missing_source)),
            Err(VerdictError::MissingField("assurance.filter_epoch.source"))
        );

        let mut missing_time = assurance();
        missing_time.watermarks[0].observed_at = None;
        assert_eq!(
            Verdict::try_from(verdict(missing_time)),
            Err(VerdictError::MissingField("watermark.observed_at"))
        );
    }

    #[test]
    fn verdict_rejects_duplicate_set_values_and_inverted_gaps() {
        let mut duplicate = assurance();
        duplicate.schema_versions.push("event.v1".to_owned());
        assert_eq!(
            Verdict::try_from(verdict(duplicate)),
            Err(VerdictError::DuplicateValue("assurance.schema_versions"))
        );

        let mut inverted = assurance();
        inverted.gaps.push(raw_model::SourceGap {
            source: Some(source()),
            sequence_epoch: 1,
            first_missing: 4,
            last_missing: 3,
            effect: "could hide a cause".to_owned(),
        });
        assert_eq!(
            Verdict::try_from(verdict(inverted)),
            Err(VerdictError::InvalidRange("assurance.gap"))
        );
    }

    #[test]
    fn verdict_decode_round_trips_through_encode_to_vec() {
        // Store/src/lib.rs (#16, ADR-0011) reads its own `verdict.payload`
        // rows back through this decoder rather than depending on `prost`
        // directly. `Verdict` had `encode_to_vec` with no matching `decode`;
        // this pins the round trip now that both exist, with no `.proto`
        // change (the type already exists in `gen/rust`).
        let original = Verdict::try_from(verdict(assurance())).expect("valid Verdict");
        let bytes = original.encode_to_vec();
        let decoded = Verdict::decode(&bytes).expect("decodes its own bytes");
        assert_eq!(decoded, original);
        assert_eq!(decoded.encode_to_vec(), bytes);
    }

    #[test]
    fn verdict_decode_rejects_garbage_bytes_and_invalid_messages() {
        assert!(matches!(
            Verdict::decode(&[0xFF, 0x00, 0xFF]),
            Err(VerdictError::Decode(_))
        ));

        // Well-formed Protobuf, but missing every required field: decodes at
        // the wire level and then fails the same domain validation
        // `TryFrom<raw_model::Verdict>` already enforces.
        let empty = raw_model::Verdict::default().encode_to_vec();
        assert_eq!(
            Verdict::decode(&empty),
            Err(VerdictError::MissingScalar("verdict.id"))
        );
    }

    /// PENDING with every present acknowledged ≥ envelope target still validates.
    ///
    /// Epoch comparison is not the definition of PENDING: watermark lag, no complete
    /// boundary, and several other `ActivationBarrier::classify` arms hold after every
    /// covered source has acknowledged the target.
    #[test]
    fn pending_barrier_validates_when_every_acknowledged_epoch_meets_the_target() {
        let mut envelope = assurance();
        envelope.activation_barrier = raw_model::ActivationBarrierStatus::Pending as i32;
        envelope.target_filter_epoch = Some(9);
        envelope.filter_epochs[0].filter_epoch = Some(9);
        envelope.cold_path = raw_model::TierSettlement::NotRequired as i32;
        envelope.settlement_basis = None;
        let mut value = verdict(envelope);
        value.authority = raw_model::VerdictAuthority::Provisional as i32;
        let decoded = Verdict::try_from(value).expect("watermark-lag PENDING must validate");
        assert_eq!(
            decoded.activation_barrier(),
            ActivationBarrierStatus::Pending
        );
        assert_eq!(decoded.target_filter_epoch(), Some(FilterEpoch::new(9)));
        assert_eq!(decoded.cold_path(), TierSettlement::NotRequired);
    }

    /// CLEARED must not lie: missing row, unset ack, or ack < target is incomplete.
    #[test]
    fn cleared_barrier_rejects_a_missing_row_unset_ack_or_lagging_ack() {
        let mut missing_row = assurance();
        missing_row.activation_barrier = raw_model::ActivationBarrierStatus::Cleared as i32;
        missing_row.target_filter_epoch = Some(9);
        missing_row.required_sources.push(source_named("ledger"));
        missing_row.observed_sources.push(source_named("ledger"));
        assert_eq!(
            Verdict::try_from(verdict(missing_row)),
            Err(VerdictError::ClearedBarrierIncomplete)
        );

        let mut unset_ack = assurance();
        unset_ack.activation_barrier = raw_model::ActivationBarrierStatus::Cleared as i32;
        unset_ack.target_filter_epoch = Some(9);
        unset_ack.filter_epochs[0].filter_epoch = None;
        assert_eq!(
            Verdict::try_from(verdict(unset_ack)),
            Err(VerdictError::ClearedBarrierIncomplete)
        );

        let mut lagging = assurance();
        lagging.activation_barrier = raw_model::ActivationBarrierStatus::Cleared as i32;
        lagging.target_filter_epoch = Some(9);
        lagging.filter_epochs[0].filter_epoch = Some(8);
        assert_eq!(
            Verdict::try_from(verdict(lagging)),
            Err(VerdictError::ClearedBarrierIncomplete)
        );
    }

    #[test]
    fn authoritative_pending_barrier_is_a_rollout_lie_not_a_tier_lie() {
        let mut envelope = assurance();
        envelope.activation_barrier = raw_model::ActivationBarrierStatus::Pending as i32;
        envelope.target_filter_epoch = Some(9);
        envelope.cold_path = raw_model::TierSettlement::Pending as i32;
        envelope.settlement_basis = None;
        assert_eq!(
            Verdict::try_from(verdict(envelope)),
            Err(VerdictError::AuthoritativeBarrierPending)
        );
    }

    #[test]
    fn undeclared_barrier_must_not_carry_a_target() {
        let mut envelope = assurance();
        envelope.target_filter_epoch = Some(9);
        assert_eq!(
            Verdict::try_from(verdict(envelope)),
            Err(VerdictError::UndeclaredBarrierHasTarget)
        );
    }

    #[test]
    fn unspecified_activation_barrier_is_rejected_by_try_from() {
        let mut envelope = assurance();
        envelope.activation_barrier = raw_model::ActivationBarrierStatus::Unspecified as i32;
        assert_eq!(
            Verdict::try_from(verdict(envelope)),
            Err(VerdictError::UnspecifiedEnum(
                "assurance.activation_barrier"
            ))
        );
    }

    fn encode_legacy(mut envelope: raw_model::AssuranceEnvelope) -> Vec<u8> {
        envelope.activation_barrier = raw_model::ActivationBarrierStatus::Unspecified as i32;
        envelope.target_filter_epoch = None;
        envelope.raw_event_retention_days = None;
        envelope.evidence_extract_retention = None;
        envelope.redaction_policy = None;
        verdict(envelope).encode_to_vec()
    }

    /// Historical `NoEvidence` warming rows decode as PENDING with the target unset.
    #[test]
    fn decode_relabels_no_evidence_warming_bytes_as_pending_without_a_target() {
        let mut envelope = assurance();
        envelope.cold_path = raw_model::TierSettlement::Pending as i32;
        envelope.settlement_basis = None;
        envelope.uncertainty = vec![raw_model::Uncertainty {
            reason: raw_model::InconclusiveReason::PrefixUncertainty as i32,
            detail: format!("{BARRIER_PENDING_DETAIL_PREFIX} app/gateway (no completeness signal)"),
        }];
        let mut value = verdict(envelope);
        value.authority = raw_model::VerdictAuthority::Provisional as i32;
        value
            .assurance
            .as_mut()
            .expect("envelope")
            .activation_barrier = raw_model::ActivationBarrierStatus::Unspecified as i32;
        let bytes = value.encode_to_vec();
        let decoded = Verdict::decode(&bytes).expect("historical NoEvidence must decode");
        assert_eq!(
            decoded.activation_barrier(),
            ActivationBarrierStatus::Pending
        );
        assert_eq!(decoded.target_filter_epoch(), None);
        assert_eq!(decoded.outcome(), VerdictOutcome::Satisfy);
        assert_eq!(decoded.authority(), VerdictAuthority::Provisional);
    }

    #[test]
    fn decode_recovers_a_unique_target_from_the_epoch_phrase() {
        let mut envelope = assurance();
        envelope.cold_path = raw_model::TierSettlement::Pending as i32;
        envelope.settlement_basis = None;
        envelope.uncertainty = vec![raw_model::Uncertainty {
            reason: raw_model::InconclusiveReason::PrefixUncertainty as i32,
            detail: format!(
                "{BARRIER_PENDING_DETAIL_PREFIX} app/gateway (acknowledged Filter epoch 3 is \
                 below the target epoch 7)"
            ),
        }];
        let mut value = verdict(envelope);
        value.authority = raw_model::VerdictAuthority::Provisional as i32;
        value
            .assurance
            .as_mut()
            .expect("envelope")
            .activation_barrier = raw_model::ActivationBarrierStatus::Unspecified as i32;
        let decoded = Verdict::decode(&value.encode_to_vec()).expect("epoch-phrase must decode");
        assert_eq!(
            decoded.activation_barrier(),
            ActivationBarrierStatus::Pending
        );
        assert_eq!(decoded.target_filter_epoch(), Some(FilterEpoch::new(7)));
    }

    #[test]
    fn decode_inserts_a_silent_required_source_row_visible_to_filter_epoch_rows() {
        let mut envelope = assurance();
        envelope.cold_path = raw_model::TierSettlement::Pending as i32;
        envelope.settlement_basis = None;
        envelope.required_sources.push(source_named("ledger"));
        envelope.source_tier_gap = Some(raw_model::SourceTierGap {
            contributing_sources: Vec::new(),
            detail: "required source declared no completeness signal".to_owned(),
            unanchored_sources: vec![source_named("ledger")],
        });
        envelope.uncertainty = vec![raw_model::Uncertainty {
            reason: raw_model::InconclusiveReason::PrefixUncertainty as i32,
            detail: format!("{BARRIER_PENDING_DETAIL_PREFIX} app/ledger (no completeness signal)"),
        }];
        let mut value = verdict(envelope);
        value.authority = raw_model::VerdictAuthority::Provisional as i32;
        value
            .assurance
            .as_mut()
            .expect("envelope")
            .activation_barrier = raw_model::ActivationBarrierStatus::Unspecified as i32;
        let decoded = Verdict::decode(&value.encode_to_vec()).expect("silent source must decode");
        let silent = source_from_wire(&source_named("ledger")).expect("validated source");
        assert_eq!(decoded.filter_epochs().get(&silent), None);
        assert_eq!(decoded.filter_epoch_rows().get(&silent), Some(&None));
    }

    #[test]
    fn decode_relabels_pre_164_authoritative_bytes_as_not_declared() {
        let bytes = encode_legacy(assurance());
        let decoded = Verdict::decode(&bytes).expect("pre-#164 Authoritative must decode");
        assert_eq!(
            decoded.activation_barrier(),
            ActivationBarrierStatus::NotDeclared
        );
        assert_eq!(decoded.target_filter_epoch(), None);
        assert_eq!(decoded.authority(), VerdictAuthority::Authoritative);
    }

    #[test]
    fn decode_relabel_is_idempotent_when_field_22_is_already_set() {
        let mut envelope = assurance();
        envelope.activation_barrier = raw_model::ActivationBarrierStatus::Pending as i32;
        envelope.target_filter_epoch = Some(4);
        envelope.settlement_basis = None;
        let mut value = verdict(envelope);
        value.authority = raw_model::VerdictAuthority::Provisional as i32;
        let original = Verdict::try_from(value).expect("hand-built PENDING validates");
        let decoded = Verdict::decode(&original.encode_to_vec()).expect("round-trips");
        assert_eq!(
            decoded.activation_barrier(),
            ActivationBarrierStatus::Pending
        );
        assert_eq!(decoded.target_filter_epoch(), Some(FilterEpoch::new(4)));
    }

    #[test]
    fn try_from_rejects_missing_raw_event_retention_days() {
        let mut envelope = assurance();
        envelope.raw_event_retention_days = None;
        assert_eq!(
            Verdict::try_from(verdict(envelope)),
            Err(VerdictError::MissingField(
                "assurance.raw_event_retention_days"
            ))
        );
    }

    #[test]
    fn try_from_rejects_raw_event_retention_days_out_of_bounds() {
        for days in [0_u32, 6, 366] {
            let mut envelope = assurance();
            envelope.raw_event_retention_days = Some(days);
            assert_eq!(
                Verdict::try_from(verdict(envelope)),
                Err(VerdictError::InvalidRange(
                    "assurance.raw_event_retention_days"
                ))
            );
        }
    }

    #[test]
    fn try_from_rejects_unset_evidence_extract_retention_oneof() {
        let mut envelope = assurance();
        envelope.evidence_extract_retention = None;
        assert_eq!(
            Verdict::try_from(verdict(envelope)),
            Err(VerdictError::MissingField(
                "assurance.evidence_extract_retention"
            ))
        );
    }

    #[test]
    fn try_from_accepts_not_deployed_evidence_extract_retention() {
        let envelope = assurance();
        let decoded = Verdict::try_from(verdict(envelope)).expect("NotDeployed");
        assert_eq!(
            decoded.evidence_extract_retention(),
            RetentionWindow::NotDeployed
        );
    }

    #[test]
    fn try_from_rejects_evidence_extract_retention_days_out_of_bounds() {
        for days in [0_u32, 366, u32::MAX] {
            let mut envelope = assurance();
            envelope.evidence_extract_retention = Some(
                raw_model::assurance_envelope::EvidenceExtractRetention::EvidenceExtractRetentionDays(
                    days,
                ),
            );
            assert_eq!(
                Verdict::try_from(verdict(envelope)),
                Err(VerdictError::InvalidRange(
                    "assurance.evidence_extract_retention_days"
                ))
            );
        }
    }

    #[test]
    fn try_from_accepts_evidence_extract_retention_days_in_bounds() {
        // 1 / configured-default 90 / 365; 3 locks `1..=6` still legal (shape-only
        // analogy to raw min 7); 12 is the pre-existing configured fixture value.
        for days in [1_u32, 3, 12, 90, 365] {
            let mut envelope = assurance();
            envelope.evidence_extract_retention = Some(
                raw_model::assurance_envelope::EvidenceExtractRetention::EvidenceExtractRetentionDays(
                    days,
                ),
            );
            let decoded = Verdict::try_from(verdict(envelope)).expect("in-bounds days");
            assert_eq!(
                decoded.evidence_extract_retention(),
                RetentionWindow::Days(EvidenceExtractRetentionDays::try_new(days).expect("days"))
            );
        }
    }

    #[test]
    fn try_from_accepts_evidence_extract_days_greater_than_raw_event_days() {
        // ADR-0026 §8: extract plaintext may outlive redacted/aged raw Events —
        // no per-envelope ≤ coupling.
        let mut envelope = assurance();
        envelope.raw_event_retention_days = Some(30);
        envelope.evidence_extract_retention = Some(
            raw_model::assurance_envelope::EvidenceExtractRetention::EvidenceExtractRetentionDays(
                90,
            ),
        );
        let decoded = Verdict::try_from(verdict(envelope)).expect("extract > raw");
        assert_eq!(
            decoded.raw_event_retention_days(),
            RawEventRetentionDays::try_new(30).expect("30")
        );
        assert_eq!(
            decoded.evidence_extract_retention(),
            RetentionWindow::Days(EvidenceExtractRetentionDays::configured_default())
        );
    }

    #[test]
    fn try_from_rejects_unset_redaction_policy_oneof() {
        let mut envelope = assurance();
        envelope.redaction_policy = None;
        assert_eq!(
            Verdict::try_from(verdict(envelope)),
            Err(VerdictError::MissingField("assurance.redaction_policy"))
        );
    }

    #[test]
    fn try_from_rejects_blank_redaction_policy_version() {
        let mut envelope = assurance();
        envelope.redaction_policy = Some(
            raw_model::assurance_envelope::RedactionPolicy::RedactionPolicyVersion(String::new()),
        );
        assert_eq!(
            Verdict::try_from(verdict(envelope)),
            Err(VerdictError::MissingScalar(
                "assurance.redaction_policy_version"
            ))
        );
    }

    #[test]
    fn try_from_accepts_configured_redaction_policy_version() {
        let mut envelope = assurance();
        envelope.redaction_policy = Some(
            raw_model::assurance_envelope::RedactionPolicy::RedactionPolicyVersion(
                "policy-v3".to_owned(),
            ),
        );
        let decoded = Verdict::try_from(verdict(envelope)).expect("configured version");
        assert_eq!(
            decoded.redaction_policy(),
            RedactionPolicyVersion::Version("policy-v3".to_owned())
        );
    }

    #[test]
    fn decode_backfills_pre_206_retention_fields() {
        let bytes = encode_legacy(assurance());
        let decoded = Verdict::decode(&bytes).expect("historical envelope must decode");
        assert_eq!(
            decoded.raw_event_retention_days(),
            RawEventRetentionDays::platform_default()
        );
        assert_eq!(
            decoded.evidence_extract_retention(),
            RetentionWindow::NotDeployed
        );
        assert_eq!(
            decoded.redaction_policy(),
            RedactionPolicyVersion::NoPolicyConfigured
        );
    }

    #[test]
    fn decode_retention_backfill_is_idempotent_when_fields_already_set() {
        let mut envelope = assurance();
        envelope.raw_event_retention_days = Some(45);
        envelope.evidence_extract_retention = Some(
            raw_model::assurance_envelope::EvidenceExtractRetention::EvidenceExtractRetentionDays(
                12,
            ),
        );
        envelope.redaction_policy = Some(
            raw_model::assurance_envelope::RedactionPolicy::RedactionPolicyVersion(
                "keep-me".to_owned(),
            ),
        );
        let decoded = Verdict::decode(&verdict(envelope).encode_to_vec()).expect("round-trips");
        assert_eq!(
            decoded.raw_event_retention_days(),
            RawEventRetentionDays::try_new(45).expect("45")
        );
        assert_eq!(
            decoded.evidence_extract_retention(),
            RetentionWindow::Days(EvidenceExtractRetentionDays::try_new(12).expect("12"))
        );
        assert_eq!(
            decoded.redaction_policy(),
            RedactionPolicyVersion::Version("keep-me".to_owned())
        );
    }

    #[test]
    fn decode_legacy_without_field_29_yields_empty_redacted_attributes() {
        // Proto3 repeated defaults: omitting field 29 (empty vec on encode) decodes as [].
        let bytes = verdict(assurance()).encode_to_vec();
        let decoded = Verdict::decode(&bytes).expect("current envelope");
        assert!(decoded.redacted_attributes().is_empty());
        assert_eq!(
            decoded.redaction_policy(),
            RedactionPolicyVersion::NoPolicyConfigured
        );
    }

    #[test]
    fn try_from_rejects_non_empty_redacted_attributes_under_none_policy() {
        let mut envelope = assurance();
        envelope.redacted_attributes = vec![raw_model::RedactedAttributeRef {
            event_id: "event".to_owned(),
            attribute_path: "attrs.ssn".to_owned(),
        }];
        assert_eq!(
            Verdict::try_from(verdict(envelope)),
            Err(VerdictError::RedactedAttributesWithoutPolicy)
        );
    }

    #[test]
    fn try_from_accepts_empty_redacted_attributes_under_configured_version() {
        let mut envelope = assurance();
        envelope.redaction_policy = Some(
            raw_model::assurance_envelope::RedactionPolicy::RedactionPolicyVersion(
                "policy-v3".to_owned(),
            ),
        );
        envelope.redacted_attributes = Vec::new();
        let decoded = Verdict::try_from(verdict(envelope)).expect("empty under version");
        assert!(decoded.redacted_attributes().is_empty());
        assert_eq!(
            decoded.redaction_policy(),
            RedactionPolicyVersion::Version("policy-v3".to_owned())
        );
    }

    #[test]
    fn try_from_accepts_valid_redacted_attributes_under_configured_version() {
        let mut envelope = assurance();
        envelope.redaction_policy = Some(
            raw_model::assurance_envelope::RedactionPolicy::RedactionPolicyVersion(
                "policy-v3".to_owned(),
            ),
        );
        envelope.redacted_attributes = vec![
            raw_model::RedactedAttributeRef {
                event_id: "event-b".to_owned(),
                attribute_path: "attrs.email".to_owned(),
            },
            raw_model::RedactedAttributeRef {
                event_id: "event-a".to_owned(),
                attribute_path: "attrs.ssn".to_owned(),
            },
        ];
        let decoded = Verdict::try_from(verdict(envelope)).expect("valid inventory");
        assert_eq!(
            decoded.redacted_attributes(),
            vec![
                RedactedAttributeRef {
                    event_id: "event-a".to_owned(),
                    attribute_path: "attrs.ssn".to_owned(),
                },
                RedactedAttributeRef {
                    event_id: "event-b".to_owned(),
                    attribute_path: "attrs.email".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn try_from_rejects_duplicate_redacted_attribute_pair() {
        let mut envelope = assurance();
        envelope.redaction_policy = Some(
            raw_model::assurance_envelope::RedactionPolicy::RedactionPolicyVersion(
                "policy-v3".to_owned(),
            ),
        );
        envelope.redacted_attributes = vec![
            raw_model::RedactedAttributeRef {
                event_id: "event".to_owned(),
                attribute_path: "attrs.ssn".to_owned(),
            },
            raw_model::RedactedAttributeRef {
                event_id: "event".to_owned(),
                attribute_path: "attrs.ssn".to_owned(),
            },
        ];
        assert_eq!(
            Verdict::try_from(verdict(envelope)),
            Err(VerdictError::DuplicateValue(
                "assurance.redacted_attributes"
            ))
        );
    }

    #[test]
    fn try_from_rejects_blank_redacted_attribute_event_id() {
        let mut envelope = assurance();
        envelope.redaction_policy = Some(
            raw_model::assurance_envelope::RedactionPolicy::RedactionPolicyVersion(
                "policy-v3".to_owned(),
            ),
        );
        envelope.redacted_attributes = vec![raw_model::RedactedAttributeRef {
            event_id: String::new(),
            attribute_path: "attrs.ssn".to_owned(),
        }];
        assert_eq!(
            Verdict::try_from(verdict(envelope)),
            Err(VerdictError::MissingScalar(
                "assurance.redacted_attributes.event_id"
            ))
        );
    }

    #[test]
    fn try_from_rejects_blank_redacted_attribute_path() {
        let mut envelope = assurance();
        envelope.redaction_policy = Some(
            raw_model::assurance_envelope::RedactionPolicy::RedactionPolicyVersion(
                "policy-v3".to_owned(),
            ),
        );
        envelope.redacted_attributes = vec![raw_model::RedactedAttributeRef {
            event_id: "event".to_owned(),
            attribute_path: "  ".to_owned(),
        }];
        assert_eq!(
            Verdict::try_from(verdict(envelope)),
            Err(VerdictError::MissingScalar(
                "assurance.redacted_attributes.attribute_path"
            ))
        );
    }

    #[test]
    fn canonicalize_orders_redacted_attributes_by_event_id_then_path() {
        let mut envelope = assurance();
        envelope.redaction_policy = Some(
            raw_model::assurance_envelope::RedactionPolicy::RedactionPolicyVersion(
                "policy-v3".to_owned(),
            ),
        );
        envelope.redacted_attributes = vec![
            raw_model::RedactedAttributeRef {
                event_id: "evt-2".to_owned(),
                attribute_path: "b".to_owned(),
            },
            raw_model::RedactedAttributeRef {
                event_id: "evt-1".to_owned(),
                attribute_path: "z".to_owned(),
            },
            raw_model::RedactedAttributeRef {
                event_id: "evt-1".to_owned(),
                attribute_path: "a".to_owned(),
            },
        ];
        let first = Verdict::try_from(verdict(envelope.clone())).expect("first");
        envelope.redacted_attributes.reverse();
        let second = Verdict::try_from(verdict(envelope)).expect("reversed input");
        assert_eq!(first.redacted_attributes(), second.redacted_attributes());
        assert_eq!(
            first.redacted_attributes(),
            vec![
                RedactedAttributeRef {
                    event_id: "evt-1".to_owned(),
                    attribute_path: "a".to_owned(),
                },
                RedactedAttributeRef {
                    event_id: "evt-1".to_owned(),
                    attribute_path: "z".to_owned(),
                },
                RedactedAttributeRef {
                    event_id: "evt-2".to_owned(),
                    attribute_path: "b".to_owned(),
                },
            ]
        );
        assert_eq!(first.encode_to_vec(), second.encode_to_vec());
    }

    #[test]
    fn core_shaped_envelope_with_empty_inventory_is_try_from_green() {
        // Mirrors Core settle-time builders: RedactionPolicyNone + empty inventory.
        let envelope = assurance();
        assert!(envelope.redacted_attributes.is_empty());
        assert!(matches!(
            envelope.redaction_policy.as_ref(),
            Some(raw_model::assurance_envelope::RedactionPolicy::RedactionPolicyNone(_))
        ));
        let decoded = Verdict::try_from(verdict(envelope)).expect("Core-shaped empty inventory");
        assert!(decoded.redacted_attributes().is_empty());
    }
}
