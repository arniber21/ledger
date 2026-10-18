//! Validation and canonicalization shared by more than one generated protocol boundary.
//!
//! Event Filters (ADR-0006) and Verdicts (ADR-0022) both carry source identities and
//! Watermarks, and both must canonicalize them into one byte-identical ordering so that
//! encoded messages compare by value. The helpers here are the single implementation of
//! those rules; each boundary keeps its own error taxonomy and converts from [`WireError`].
//!
//! The encoders published here are the exception to the module's otherwise `pub(crate)`
//! surface. Two criteria admit an encoder, and they are genuinely two criteria rather than
//! one restated:
//!
//! 1. **Downstream duplication proves demand.** [`SourceIdentity::to_wire`],
//!    [`SourceTier::to_raw`], [`SourceCapability::to_raw`] and
//!    [`ObservationTime::to_wire`] were published because a crate outside this one had
//!    rebuilt them by hand, and a hand-rebuilt copy of a `#[non_exhaustive]` enum's
//!    encoder is forced by rustc into a wildcard arm that silently mislabels a variant
//!    added later.
//!
//!    [`ObservationTime::to_wire`] is the one member of that set that is not a `match`,
//!    and the difference sharpens the criterion rather than straining it. The other
//!    three **construct** a message out of validated parts, so a copy of them can be
//!    wrong about a variant. This one **hands back** the message the domain type is
//!    already holding, so it is the only member for which no re-derivation exists to be
//!    wrong about -- and `sentinel-core` had written the re-derivation anyway (#112).
//! 2. **A downstream crate needs the variant identity as a *match scrutinee*.**
//!    [`Integrity::to_raw`] is published for this reason and no other -- nothing anywhere
//!    duplicates it. This criterion **extends** criterion 1; it does not fit inside it, and
//!    claiming otherwise would be false. `sentinel-store` cannot exhaustively match a
//!    `#[non_exhaustive]` domain enum, but it can exhaustively match the closed generated
//!    enum, and `to_raw` is this crate's own compile-checked bridge to it. Publishing
//!    `to_raw` is what lets a downstream crate turn "a variant was added" from a silent
//!    fallback into a compile error in its own file. See `store/src/codec.rs`.
//!
//!    [`VerdictOutcome::to_raw`], [`VerdictAuthority::to_raw`],
//!    [`InconclusiveReason::to_raw`] and [`TierSettlement::to_raw`] are published under
//!    the same criterion, and for a sharper reason than [`Integrity::to_raw`] was: the
//!    wildcard they replace is not merely an ordering claim about unseen variants, it
//!    **already defeats a live guard**. `store/src/postgres/verdicts.rs`'s ADR-0021 SA3
//!    check refuses walking an `Authoritative` Verdict back to `Provisional`, and the old
//!    `encode_verdict_authority` mapped *every* unrecognized variant to `"provisional"`.
//!    So an `Authoritative -> X` write landed exactly the transition SA3 exists to refuse,
//!    while the guard's `==` did not fire because `X` is not literally `Provisional`. A
//!    `Delegated` variant added in an isolated tree compiled clean with every test green
//!    (#133).
//!
//! The crate's remaining wire encoders stay private until one of those two applies.

use std::collections::BTreeSet;

use crate::canonical::source_from_wire;
use crate::wire::model::v1 as raw_model;
use crate::{
    ActivationBarrierStatus, InconclusiveReason, Integrity, ObservationTime, SourceCapability,
    SourceIdentity, SourceTier, TierSettlement, VerdictAuthority, VerdictOutcome,
};

/// Canonical owned identity of a source: `(monitored_application_id, source_id)`.
pub(crate) type SourceKey = (String, String);

impl SourceIdentity {
    /// Encodes this identity into its canonical wire form.
    ///
    /// This is the crate's single encoder for the type: `sentinel-core` and
    /// `sentinel-compile` must call it rather than rebuilding the message out of
    /// [`Self::monitored_application`] and [`Self::source`], which is what they did
    /// while it was private. The returned value is wire storage with only wire-level
    /// validity (ADR-0024), not a validated domain value.
    ///
    /// Publishing the egress direction confers no capability that the crate did not
    /// already grant: [`raw_model::SourceIdentity`] is generated, `pub`, and has `pub`
    /// fields, so any caller could always assemble one by hand. ADR-0024 is silent on
    /// egress; what it forbids is treating a decoded message as evidence that a value is
    /// canonical. So the inverse direction stays private, and untrusted input still
    /// enters only through the whole-message validating boundaries, each of which
    /// re-validates every nested identity it carries.
    #[must_use]
    pub fn to_wire(&self) -> raw_model::SourceIdentity {
        raw_model::SourceIdentity {
            monitored_application_id: self.monitored_application().to_string(),
            source_id: self.source().to_string(),
        }
    }
}

impl ObservationTime {
    /// Hands back this observation time's canonical wire message.
    ///
    /// This is the crate's single encoder for the type: `sentinel-core` must call it
    /// rather than rebuilding the message out of [`Self::sentinel_clock`] and
    /// [`Self::sentinel_nanoseconds`], which is what it did while this was private --
    /// `core/src/verdict.rs` carried its own `observation_to_wire`, deleted by #112. The
    /// returned value is wire storage with only wire-level validity (ADR-0024), not a
    /// validated domain value.
    ///
    /// **It is a hand-back, not a re-encode, and that is the reason it exists.** The
    /// message returned is the one [`Self::new`] built or `try_from_wire` accepted, so a
    /// field added to `ObservationTime` in the proto later reaches every caller with no
    /// caller edited. The re-derivation this replaces had the opposite property: a new
    /// field makes its struct literal fail to compile with `E0063`, and the cheapest
    /// edit that satisfies rustc is to supply the field's zero -- which compiles, leaves
    /// every test green, and silently drops whatever the Event actually carried. That
    /// `E0063` now lands in this crate's own test module, where the proto author is
    /// already looking, on `to_wire_splits_the_nanoseconds_into_the_words_the_wire_names`.
    ///
    /// Publishing the egress direction confers no capability the crate did not already
    /// grant. [`raw_model::ObservationTime`] is generated, `pub` and has `pub` fields,
    /// and its two fields are both readable through this type's existing public
    /// accessors, losslessly in each direction. The `Option` wrapping `nanoseconds` is
    /// the one place where a hand-back could disclose a state the accessors flatten --
    /// `None` and `Some(default)` are different bytes and the same accessor output --
    /// and `try_from_wire` rejects `None` with `MissingField`, so no `ObservationTime`
    /// carrying one is ever constructed. ADR-0024's constraint is on ingress: a decoded
    /// message is never evidence that a value is canonical. So `try_from_wire` stays
    /// `pub(crate)` and untrusted input still enters only through the whole-message
    /// validating boundaries.
    ///
    /// `int128_to_wire`, the split that builds this message's `nanoseconds`, stays
    /// private. Publishing [`Self::to_wire`] removes its only caller outside this crate,
    /// and a public item with no caller is exactly the speculative generality this
    /// module's publish-on-demonstrated-demand criteria exist to refuse.
    ///
    /// **`as_wire(&self) -> &raw_model::ObservationTime` was weighed and rejected**, and
    /// the rejection gives something up that is worth naming rather than eliding. It is
    /// the only signature under which "hand-back, not re-derivation" would be
    /// *compiler-checked*, since a function returning `&T` cannot return a reference to a
    /// value it has just built. Against that: it buys no clone, because both call sites
    /// need an owned message and [`crate::SourceCompletenessSignal::observed_at`] already
    /// clones on the way out; it freezes this crate into storing a
    /// [`raw_model::ObservationTime`] verbatim forever; and handing out a reference
    /// *into* the `pub(crate)` inner is a larger ADR-0024 concession than handing out a
    /// detached copy. `into_wire(self)` is the only variant that removes a clone, and it
    /// breaks every `&ObservationTime` caller -- `sentinel-store`'s
    /// `VerdictRecord::recorded_at` already is one.
    /// `to_wire_returns_the_message_try_from_wire_accepted_not_a_re_derivation` stands in
    /// for the guarantee the chosen signature declines to give.
    #[must_use]
    pub fn to_wire(&self) -> raw_model::ObservationTime {
        self.0.clone()
    }
}

impl SourceTier {
    /// Encodes this tier into its generated proto enum value.
    ///
    /// The match is exhaustive, which is the point: [`SourceTier`] is
    /// `#[non_exhaustive]`, so a copy of this encoder outside the crate is forced by
    /// rustc into a wildcard arm, and a tier added later would be encoded as whatever
    /// that arm names instead of failing to compile. ADR-0022 bounds a Verdict's
    /// trustworthiness by the tier of the Events anchoring it, so a mislabelled tier is
    /// an assurance defect in the externally visible Assurance Envelope. Encoding only.
    #[must_use]
    pub const fn to_raw(self) -> i32 {
        match self {
            Self::Anchor => raw_model::SourceTier::Anchor as i32,
            Self::Contributing => raw_model::SourceTier::Contributing as i32,
        }
    }
}

impl SourceCapability {
    /// Encodes this capability into its generated proto enum value.
    ///
    /// Exhaustive for the same reason as [`SourceTier::to_raw`]: [`SourceCapability`] is
    /// `#[non_exhaustive]`, so an out-of-crate copy must carry a wildcard, and the
    /// honest thing to put there is a panic. Keeping the single encoder here turns that
    /// runtime panic back into the compile error it should have been. Encoding only.
    #[must_use]
    pub const fn to_raw(self) -> i32 {
        match self {
            Self::ObserveBeforeEffect => raw_model::SourceCapability::ObserveBeforeEffect as i32,
            Self::CausalEdges => raw_model::SourceCapability::CausalEdges as i32,
            Self::CompletenessSignals => raw_model::SourceCapability::CompletenessSignals as i32,
            Self::BoundedClockUncertainty => {
                raw_model::SourceCapability::BoundedClockUncertainty as i32
            }
        }
    }
}

impl Integrity {
    /// Encodes this integrity level into its generated proto enum value.
    ///
    /// Exhaustive for the same reason as [`SourceTier::to_raw`], but published for a
    /// *different* reason, and the difference is worth stating rather than eliding.
    /// Criterion 1 of this module's header -- "downstream duplication proves demand" --
    /// does **not** apply: nothing outside this crate has ever duplicated this mapping,
    /// and `store::codec::encode_integrity` is a different mapping to a different
    /// alphabet (SQL column text), not a copy of this one.
    ///
    /// It is published under criterion 2: `sentinel-store` needs the *variant identity* as
    /// a **match scrutinee**. `store::codec` cannot exhaustively match the
    /// `#[non_exhaustive]` domain [`Integrity`], so it matched with a wildcard and mapped
    /// every unrecognized variant to the `"unverified"` column text. That is an ordering
    /// claim -- "least trust" -- about variants the matching build cannot see, and it is
    /// wrong in a specific direction: it understates a hypothetical `AttestedHardware`
    /// harmlessly, but it launders a hypothetical `KnownTampered` into ordinary
    /// absence-of-evidence. Routing through this function lets `store` match the closed
    /// generated enum instead, so the arm becomes a compile error there. Encoding only;
    /// the inverse stays private, per ADR-0024.
    #[must_use]
    pub const fn to_raw(self) -> i32 {
        match self {
            Self::Unverified => raw_model::Integrity::Unverified as i32,
            Self::Authenticated => raw_model::Integrity::Authenticated as i32,
            Self::TamperEvident => raw_model::Integrity::TamperEvident as i32,
        }
    }
}

impl VerdictOutcome {
    /// Encodes this outcome into its generated proto enum value.
    ///
    /// Exhaustive for the reason given on [`SourceTier::to_raw`], and published under this
    /// module's criterion 2 so `store::codec` can match the closed generated enum. The
    /// wildcard it replaces mapped every unrecognized variant to `"inconclusive"` — the
    /// safe direction, but a claim about variants the matching build cannot see.
    #[must_use]
    pub const fn to_raw(self) -> i32 {
        match self {
            Self::Satisfy => raw_model::VerdictOutcome::Satisfy as i32,
            Self::Violate => raw_model::VerdictOutcome::Violate as i32,
            Self::Inconclusive => raw_model::VerdictOutcome::Inconclusive as i32,
        }
    }
}

impl VerdictAuthority {
    /// Encodes this authority into its generated proto enum value.
    ///
    /// This is the one whose absence was demonstrably harmful rather than latent; see this
    /// module's header for the SA3 interaction. Exhaustive here means a new authority is a
    /// compile error in this file, which is the only place that can see every variant.
    #[must_use]
    pub const fn to_raw(self) -> i32 {
        match self {
            Self::Provisional => raw_model::VerdictAuthority::Provisional as i32,
            Self::Authoritative => raw_model::VerdictAuthority::Authoritative as i32,
        }
    }
}

impl InconclusiveReason {
    /// Encodes this reason into its generated proto enum value.
    ///
    /// Exhaustive for the reason given on [`SourceTier::to_raw`]. The wildcard it replaces
    /// recorded an unknown reason as `"unrecognized"` rather than mapping it onto one of
    /// the three named causes, which was the right fallback — but a fallback is still a
    /// runtime answer to a question the compiler can answer.
    #[must_use]
    pub const fn to_raw(self) -> i32 {
        match self {
            Self::PrefixUncertainty => raw_model::InconclusiveReason::PrefixUncertainty as i32,
            Self::ObservationalUncertainty => {
                raw_model::InconclusiveReason::ObservationalUncertainty as i32
            }
            Self::EpistemicUncertainty => {
                raw_model::InconclusiveReason::EpistemicUncertainty as i32
            }
            Self::AnchorSilence => raw_model::InconclusiveReason::AnchorSilence as i32,
        }
    }
}

impl ActivationBarrierStatus {
    /// Encodes this discriminator into its generated proto enum value.
    ///
    /// Exhaustive for the reason given on [`SourceTier::to_raw`]. `UNSPECIFIED` is
    /// not constructible on the domain type.
    #[must_use]
    pub const fn to_raw(self) -> i32 {
        match self {
            Self::NotDeclared => raw_model::ActivationBarrierStatus::NotDeclared as i32,
            Self::Pending => raw_model::ActivationBarrierStatus::Pending as i32,
            Self::Cleared => raw_model::ActivationBarrierStatus::Cleared as i32,
        }
    }
}

impl TierSettlement {
    /// Encodes this settlement state into its generated proto enum value.
    ///
    /// Exhaustive for the reason given on [`SourceTier::to_raw`]. The wildcard it replaces
    /// mapped unknown states to `"pending"`, which understates settlement rather than
    /// overstating it — safe, and still a claim this crate is the only place able to check.
    #[must_use]
    pub const fn to_raw(self) -> i32 {
        match self {
            Self::NotRequired => raw_model::TierSettlement::NotRequired as i32,
            Self::Pending => raw_model::TierSettlement::Pending as i32,
            Self::Settled => raw_model::TierSettlement::Settled as i32,
        }
    }
}

/// Validation failure in a wire shape shared by more than one protocol boundary.
///
/// Never escapes the crate: each boundary converts it into its own error type, so the
/// variants here are exactly those the shared helpers below can produce.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum WireError {
    /// Required message is absent.
    MissingField(&'static str),
    /// A set-like repeated value is duplicated.
    DuplicateValue(&'static str),
    /// Nested canonical-model evidence is invalid.
    InvalidEvidence(crate::EventError),
}

pub(crate) fn validate_source(source: &raw_model::SourceIdentity) -> Result<(), WireError> {
    source_from_wire(source)
        .map(|_| ())
        .map_err(WireError::InvalidEvidence)
}

pub(crate) fn source_key(source: &raw_model::SourceIdentity) -> (&str, &str) {
    (&source.monitored_application_id, &source.source_id)
}

pub(crate) fn owned_source_key(source: &raw_model::SourceIdentity) -> SourceKey {
    (
        source.monitored_application_id.clone(),
        source.source_id.clone(),
    )
}

pub(crate) fn compare_optional_sources(
    left: Option<&raw_model::SourceIdentity>,
    right: Option<&raw_model::SourceIdentity>,
) -> std::cmp::Ordering {
    left.map(source_key).cmp(&right.map(source_key))
}

pub(crate) fn sort_sources(sources: &mut [raw_model::SourceIdentity]) {
    sources.sort_by(|left, right| source_key(left).cmp(&source_key(right)));
}

pub(crate) fn sort_watermarks(watermarks: &mut [raw_model::Watermark]) {
    watermarks.sort_by(|left, right| {
        compare_optional_sources(left.source.as_ref(), right.source.as_ref())
            .then_with(|| left.sequence_epoch.cmp(&right.sequence_epoch))
            .then_with(|| left.complete_through.cmp(&right.complete_through))
    });
}

fn validate_observation_time(
    observed_at: Option<raw_model::ObservationTime>,
    field: &'static str,
) -> Result<(), WireError> {
    let observed_at = observed_at.ok_or(WireError::MissingField(field))?;
    ObservationTime::try_from_wire(observed_at)
        .map(|_| ())
        .map_err(WireError::InvalidEvidence)
}

pub(crate) fn validate_watermarks(
    watermarks: &[raw_model::Watermark],
    field: &'static str,
) -> Result<BTreeSet<(String, String)>, WireError> {
    let mut sources = BTreeSet::new();
    for watermark in watermarks {
        let source = watermark
            .source
            .as_ref()
            .ok_or(WireError::MissingField("watermark.source"))?;
        validate_source(source)?;
        let key = (
            source.monitored_application_id.clone(),
            source.source_id.clone(),
        );
        if !sources.insert(key) {
            return Err(WireError::DuplicateValue(field));
        }
        validate_observation_time(watermark.observed_at.clone(), "watermark.observed_at")?;
    }
    Ok(sources)
}

#[cfg(test)]
mod tests {
    use super::{
        raw_model, ActivationBarrierStatus, InconclusiveReason, Integrity, SourceCapability,
        SourceTier, TierSettlement, VerdictAuthority, VerdictOutcome,
    };

    /// Every declared proto variant of `$raw`, discovered rather than listed.
    ///
    /// `try_from` succeeds exactly on declared variants, so this enumerates the
    /// generated enum without a hand-kept list that the next author can forget
    /// to extend. The scan is bounded because every enum value under
    /// `proto/sentinel/model/v1/` is a small non-negative integer -- a property
    /// of the package's numbering convention rather than of any one file, said
    /// that way so the bound does not read as stale the first time this pattern
    /// is pointed at an enum outside `common.proto`. Negatives are included
    /// because protobuf enum values are `int32` and may legally be negative.
    ///
    /// A value outside the window is invisible here. That is a bounded blind
    /// spot rather than a hole: `assert_bijective`'s length check still fires
    /// when a `to_raw` arm names such a value, and a proto variant that no
    /// `to_raw` arm names cannot reach `store::codec`'s encoders at all. The
    /// only cost is that the length check has to offer this as an alternative
    /// cause, which it does.
    macro_rules! declared_proto {
        ($raw:path) => {
            (-1024..1024).filter_map(|v| <$raw>::try_from(v).ok().map(|r| (v, r)))
        };
    }

    /// Declares a domain enum's complete variant list, and makes rustc keep it
    /// complete.
    ///
    /// Stable Rust cannot enumerate an enum's variants, so the list is
    /// hand-written. It is written *once*: this macro expands the same tokens
    /// into both the `$name` slice the assertions iterate and a `$witness`
    /// function whose `match` is exhaustive over the domain enum. Adding a
    /// variant to the enum therefore fails `$witness` to compile (`E0004`), and
    /// the only place to add the missing arm is the invocation below, which
    /// extends `$name` in the same edit. That is what makes `assert_bijective`'s
    /// "every domain variant" quantifier real rather than aspirational.
    ///
    /// `#[non_exhaustive]` does not prevent this. It constrains matches in
    /// *other* crates; these enums are declared in this one, which is also why
    /// `to_raw` can be exhaustive at all.
    ///
    /// The enforcement is a compile error in the **test** build, not the library
    /// build: `cargo build -p sentinel-model` does not compile this module. See
    /// `assert_bijective`'s residual-hole section.
    macro_rules! domain_list {
        ($name:ident, $witness:ident, $ty:ty, [$($variant:path),+ $(,)?]) => {
            /// Identity through a match that is exhaustive over the domain
            /// enum: the completeness witness for the list it builds.
            ///
            /// **If rustc sent you here,** a variant was added to the domain
            /// enum. Do not add an arm -- there is no arm to add, the arms are
            /// generated from the list in the `domain_list!` invocation below.
            /// Add the variant *there*, and give it a **new** proto variant in
            /// `to_raw`. Pointing its `to_raw` arm at an existing proto variant,
            /// or at `UNSPECIFIED`, is exactly what these tests reject.
            ///
            /// **Ignore rustc's `help`.** E0004 offers "a wildcard pattern or
            /// an explicit pattern as shown" and spans the suggestion at the
            /// arm list below -- *inside* this macro definition, three lines
            /// under the sentence you are reading. The explicit form it prints
            /// does not compile (the path it suggests is not in scope at the
            /// definition site), but the wildcard form does, and it disables
            /// this witness for **all seven** enums at once. Verified rather
            /// than assumed: with `_ => value` added below, a domain variant
            /// absent from its list, and that variant's `to_raw` arm pointing
            /// at `UNSPECIFIED`, both `cargo test -p sentinel-model` and
            /// `cargo test -p sentinel-store` pass while
            /// `store::codec::encode_source_tier` panics. The only leg that
            /// notices is `cargo clippy -- -D warnings`, through
            /// `unreachable_pattern` on the *other* six lists -- which depends
            /// on those still being complete, so it is a side effect of there
            /// being more than one enum, not a guarantee. #133 raised the count
            /// from three to seven, which strengthens that side effect and
            /// still does not turn it into a guarantee.
            const fn $witness(value: $ty) -> $ty {
                match value {
                    $($variant => $variant,)+
                }
            }

            /// Every variant of the domain enum, kept complete by the witness
            /// above.
            const $name: &[$ty] = &[$($witness($variant)),+];
        };
    }

    /// Asserts `to_raw` is a bijection from `$domain` onto the declared,
    /// non-`UNSPECIFIED` proto variants of `$raw`.
    ///
    /// # What rustc gives, and what it does not
    ///
    /// `to_raw`'s match is exhaustive over the domain enum, so the compiler
    /// forces an *arm* to exist for every domain variant. It does not force that
    /// arm to name anything in particular, and two ways of getting it wrong
    /// compile clean:
    ///
    /// - `Self::Shadow => raw_model::SourceTier::Contributing as i32` -- two
    ///   domain tiers encode as one proto tier, and
    ///   `store::codec::encode_source_tier` then writes one column text for
    ///   both. That is the ADR-0022 assurance defect this seam exists to
    ///   prevent.
    /// - `Self::Shadow => raw_model::SourceTier::Unspecified as i32` -- proto3's
    ///   mandatory zero, which `from_raw` rejects on ingress and which
    ///   `store::codec::tier_text` maps to `None`, so `encode_source_tier` would
    ///   **panic** on a durable write path.
    ///
    /// # What this rejects
    ///
    /// Scanning from the proto side gives an exact expected count per declared
    /// variant: **zero** domain variants may map to `UNSPECIFIED`, and exactly
    /// one to every other declared variant. A final length check requires
    /// `$domain` to have no members left over, which is what catches an arm
    /// pointing at a value the proto does not declare at all -- or declares
    /// outside `declared_proto!`'s scan window, which is why that check's
    /// message names both causes. Both mistakes above fail here, as does a
    /// proto variant added with nothing mapping to it.
    ///
    /// # Why counting is not enough, and what the last loop adds
    ///
    /// A **permutation** is a bijection. `Self::Satisfy => VIOLATE` together
    /// with `Self::Violate => SATISFY` leaves every count at exactly one,
    /// satisfies the length check, and relabels both values everywhere
    /// `to_raw` is called. Nothing above can see it, because every assertion
    /// above is about *how many* domain variants name a proto variant and
    /// none is about *which*.
    ///
    /// So the final loop checks the pairing, against the one thing in this
    /// crate that is not a restatement of `to_raw`: its own ingress decoder.
    /// `from_raw` is an independently written match in another file
    /// (`model/src/verdict.rs`, `model/src/event.rs`), and
    /// `from_raw(to_raw(d)) == Some(d)` is the property `sentinel-core`
    /// actually leans on -- `core/src/verdict.rs` builds a wire Verdict out of
    /// `to_raw` and hands it straight to `Verdict::try_from`, which decodes it
    /// with `from_raw`. A permuted `to_raw` makes that pair of calls return a
    /// Verdict claiming something the Core did not conclude.
    ///
    /// Executed rather than argued, on `a1235ad` before this loop existed:
    /// swapping two arms of each of the four verdict `to_raw`s, and of
    /// [`Integrity::to_raw`], left `cargo test -p sentinel-model
    /// --all-features` **green** every time, while `sentinel-store` and
    /// `sentinel-core` failed. The crate that publishes the encoder was the
    /// one crate whose own suite could not see it wrong.
    /// [`SourceTier::to_raw`] and [`SourceCapability::to_raw`] were the two
    /// exceptions, caught by `source_enums_round_trip_every_variant`
    /// (`model/src/event.rs`), which asserts exactly this for those two and no
    /// others. This loop generalizes that test to all seven and needs no
    /// variant list of its own, because `$domain` is kept complete by a
    /// compile error rather than by remembering.
    ///
    /// # The residual hole
    ///
    /// "Every domain variant" is only as strong as `$domain`, and no test can
    /// enumerate an enum. What keeps `$domain` complete is a compile error
    /// rather than an assertion -- `domain_list!` generates the list and an
    /// exhaustive match from the same tokens -- so the property is carried by
    /// `sentinel-model`'s **test** build plus this test, never by
    /// `cargo build -p sentinel-model`, which stays green for both mistakes
    /// above. A tree whose tests are not compiled and run can still reach the
    /// panic documented in `store::codec::encode_source_tier`'s `# Panics`
    /// section -- and so can one in which the witness inside `domain_list!` has
    /// been given a wildcard arm, an edit rustc's own E0004 `help` proposes.
    /// See the witness's doc comment.
    ///
    /// This test is also not the whole guard set for that panic, only the
    /// domain-to-proto half of it. That a `store::codec` `*_text` arm yields
    /// `Some` rather than `None` is asserted by
    /// `every_generated_*_is_declared_and_round_trips` in **`sentinel-store`**'s
    /// tests, and by nothing in this crate.
    macro_rules! assert_bijective {
        ($raw:path, $unspecified:path, $domain:expr, $inverse:expr) => {{
            let domain = $domain;
            let mut claimed = 0_usize;
            for (value, raw) in declared_proto!($raw) {
                let found = domain.iter().filter(|d| d.to_raw() == value).count();
                let expected = usize::from(raw != $unspecified);
                claimed += expected;
                // The cause is picked from the branch that actually fired. A
                // fixed three-way "either/or" printed two arithmetically
                // impossible causes every time: `expected` is 0 only for
                // `UNSPECIFIED`, and `found == 0` and `found > expected`
                // cannot both hold. `assert_eq!` evaluates its format
                // arguments only on failure, so this costs nothing when green.
                assert_eq!(
                    found,
                    expected,
                    "proto variant {raw:?} ({value}) must be named by exactly \
                     {expected} domain variant(s); found {found}. {}",
                    if expected == 0 {
                        "A `to_raw` arm points at `UNSPECIFIED`, which `from_raw` rejects on \
                         ingress and which `store::codec`'s `*_text` helpers map to `None`, so \
                         the matching encoder would panic on a durable write path."
                    } else if found == 0 {
                        "No `to_raw` arm names this proto variant: either it was added to the \
                         proto with no domain variant mapping to it, or an arm that used to \
                         name it was retargeted elsewhere."
                    } else {
                        "Two or more `to_raw` arms name this one proto variant, so distinct \
                         domain variants would share a single `store::codec` column text."
                    },
                );
            }
            // The second cause is not hypothetical: executed with
            // `SOURCE_TIER_SHADOW = 5000` and every other edit correct, this is
            // the assertion that fires, and the first cause alone would send
            // the author hunting a bug that is not there.
            assert_eq!(
                domain.len(),
                claimed,
                "every domain variant must map onto a distinct declared, \
                 non-UNSPECIFIED proto variant: {} are listed but only \
                 {claimed} declared proto variants were found. Either a \
                 `to_raw` arm names a value the proto does not declare, or it \
                 names one the proto *does* declare outside `declared_proto!`'s \
                 -1024..1024 scan window -- check the `.proto` for the value \
                 before hunting for a bug in `to_raw`, because the second case \
                 is a correct change this scan cannot see.",
                domain.len(),
            );
            // Which proto variant, not how many. See this macro's doc: every
            // assertion above counts, and a swap of two arms keeps the counts.
            let inverse = $inverse;
            for &member in domain {
                assert_eq!(
                    inverse(member.to_raw()),
                    Some(member),
                    "`to_raw` must name the proto variant this crate's own \
                     `from_raw` reads back as {member:?}. Two arms are swapped: \
                     that is still a bijection onto the declared proto variants, \
                     so every assertion above passes, and every value encoded \
                     through `to_raw` is relabelled as a different one.",
                );
            }
        }};
    }

    domain_list!(
        DOMAIN_TIERS,
        witness_domain_tier,
        SourceTier,
        [SourceTier::Anchor, SourceTier::Contributing]
    );

    domain_list!(
        DOMAIN_CAPABILITIES,
        witness_domain_capability,
        SourceCapability,
        [
            SourceCapability::ObserveBeforeEffect,
            SourceCapability::CausalEdges,
            SourceCapability::CompletenessSignals,
            SourceCapability::BoundedClockUncertainty,
        ]
    );

    domain_list!(
        DOMAIN_INTEGRITIES,
        witness_domain_integrity,
        Integrity,
        [
            Integrity::Unverified,
            Integrity::Authenticated,
            Integrity::TamperEvident,
        ]
    );

    domain_list!(
        DOMAIN_VERDICT_OUTCOMES,
        witness_domain_verdict_outcome,
        VerdictOutcome,
        [
            VerdictOutcome::Satisfy,
            VerdictOutcome::Violate,
            VerdictOutcome::Inconclusive,
        ]
    );

    domain_list!(
        DOMAIN_VERDICT_AUTHORITIES,
        witness_domain_verdict_authority,
        VerdictAuthority,
        [
            VerdictAuthority::Provisional,
            VerdictAuthority::Authoritative
        ]
    );

    domain_list!(
        DOMAIN_INCONCLUSIVE_REASONS,
        witness_domain_inconclusive_reason,
        InconclusiveReason,
        [
            InconclusiveReason::PrefixUncertainty,
            InconclusiveReason::ObservationalUncertainty,
            InconclusiveReason::EpistemicUncertainty,
            InconclusiveReason::AnchorSilence,
        ]
    );

    domain_list!(
        DOMAIN_TIER_SETTLEMENTS,
        witness_domain_tier_settlement,
        TierSettlement,
        [
            TierSettlement::NotRequired,
            TierSettlement::Pending,
            TierSettlement::Settled,
        ]
    );

    domain_list!(
        DOMAIN_ACTIVATION_BARRIER_STATUSES,
        witness_domain_activation_barrier_status,
        ActivationBarrierStatus,
        [
            ActivationBarrierStatus::NotDeclared,
            ActivationBarrierStatus::Pending,
            ActivationBarrierStatus::Cleared,
        ]
    );

    #[test]
    fn to_raw_is_bijective_over_declared_proto_tiers() {
        assert_bijective!(
            raw_model::SourceTier,
            raw_model::SourceTier::Unspecified,
            DOMAIN_TIERS,
            |raw| SourceTier::from_raw(raw).ok()
        );
    }

    #[test]
    fn to_raw_is_bijective_over_declared_proto_capabilities() {
        assert_bijective!(
            raw_model::SourceCapability,
            raw_model::SourceCapability::Unspecified,
            DOMAIN_CAPABILITIES,
            |raw| SourceCapability::from_raw(raw, "capability").ok()
        );
    }

    #[test]
    fn to_raw_is_bijective_over_declared_proto_integrities() {
        assert_bijective!(
            raw_model::Integrity,
            raw_model::Integrity::Unspecified,
            DOMAIN_INTEGRITIES,
            |raw| Integrity::from_raw(raw).ok()
        );
    }

    #[test]
    fn to_raw_is_bijective_over_declared_proto_verdict_outcomes() {
        assert_bijective!(
            raw_model::VerdictOutcome,
            raw_model::VerdictOutcome::Unspecified,
            DOMAIN_VERDICT_OUTCOMES,
            |raw| {
                raw_model::VerdictOutcome::try_from(raw)
                    .ok()
                    .and_then(VerdictOutcome::from_raw)
            }
        );
    }

    /// The one of the seven whose absence was demonstrated rather than argued.
    ///
    /// Executed before this test existed: a domain-only
    /// `VerdictAuthority::Delegated` whose `to_raw` arm was merged into the
    /// `Provisional` one -- which is *clippy's own printed suggestion* for the
    /// `match_same_arms` it otherwise raises -- built clean, passed
    /// `cargo clippy --all-targets -- -D warnings` in silence, passed every
    /// `sentinel-model` and `sentinel-store` test, and made
    /// `store::codec::encode_verdict_authority(Delegated)` return
    /// `"provisional"`. That is the ADR-0021 SA3 hole #133 exists to close,
    /// reborn one layer up from the wildcard it deleted. This assertion is what
    /// now refuses it.
    #[test]
    fn to_raw_is_bijective_over_declared_proto_verdict_authorities() {
        assert_bijective!(
            raw_model::VerdictAuthority,
            raw_model::VerdictAuthority::Unspecified,
            DOMAIN_VERDICT_AUTHORITIES,
            |raw| {
                raw_model::VerdictAuthority::try_from(raw)
                    .ok()
                    .and_then(VerdictAuthority::from_raw)
            }
        );
    }

    /// `InconclusiveReason` reserves proto tag 4, and that is the reason to
    /// read `assert_bijective`'s scan direction carefully here rather than
    /// assume it.
    ///
    /// `INCONCLUSIVE_REASON_EVIDENCE_GAP = 4` was removed and reserved
    /// (`proto/sentinel/model/v1/verdict.proto`). `declared_proto!` enumerates
    /// by `try_from`, which rejects a reserved number, so 4 is simply not in
    /// the scanned set and needs no exemption. A reason re-added at 4 later
    /// would reappear in the scan and be required to have exactly one domain
    /// variant naming it, which is the correct demand.
    #[test]
    fn to_raw_is_bijective_over_declared_proto_inconclusive_reasons() {
        assert_bijective!(
            raw_model::InconclusiveReason,
            raw_model::InconclusiveReason::Unspecified,
            DOMAIN_INCONCLUSIVE_REASONS,
            |raw| {
                raw_model::InconclusiveReason::try_from(raw)
                    .ok()
                    .and_then(InconclusiveReason::from_raw)
            }
        );
    }

    #[test]
    fn to_raw_is_bijective_over_declared_proto_tier_settlements() {
        assert_bijective!(
            raw_model::TierSettlement,
            raw_model::TierSettlement::Unspecified,
            DOMAIN_TIER_SETTLEMENTS,
            |raw| {
                raw_model::TierSettlement::try_from(raw)
                    .ok()
                    .and_then(TierSettlement::from_raw)
            }
        );
    }

    #[test]
    fn to_raw_is_bijective_over_declared_proto_activation_barrier_statuses() {
        assert_bijective!(
            raw_model::ActivationBarrierStatus,
            raw_model::ActivationBarrierStatus::Unspecified,
            DOMAIN_ACTIVATION_BARRIER_STATUSES,
            |raw| {
                raw_model::ActivationBarrierStatus::try_from(raw)
                    .ok()
                    .and_then(ActivationBarrierStatus::from_raw)
            }
        );
    }
}
