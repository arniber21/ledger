//! What a Pattern instance's selectors actually matched, and whether anything
//! was measured at all.
//!
//! # The defect this exists for (issue #144)
//!
//! A Specification whose **trigger selector** matches no Event evaluates
//! cleanly, produces zero Verdicts, and reads as a pass. Every aggregation
//! written as `Iterator::all` over that empty vector returns `true`, because
//! `Iterator::all` over an empty slice is `true`. A one-character typo in an
//! Event kind, or a Candidate generated against an Intent Artifact naming a tool
//! the deployment does not actually call, therefore ships green while measuring
//! nothing — a Verdict surface with an Assurance Envelope and nothing behind it.
//!
//! `instance.rs` already names the residue it could not close and says the catch
//! *"belongs where the Verdicts are"*. Half of it belongs here instead: this
//! module answers "did anything match?" **before** a Verdict can exist, which is
//! where a Candidate is validated (ADR-0009) and where #21 and #35 will feed
//! unauthored strings into [`ParameterValue`](crate::catalogue::ParameterValue).
//!
//! # Why the answer is a census and never a judgement
//!
//! **A trigger that matched nothing is not by itself a defect.** SagaShop's
//! `cancel` capture cancels its one order before anything ships, so P4's trigger
//! matches nothing there — and the frozen oracle matrix says `"pass"` for that
//! cell, correctly: `AG(trigger -> EP(requirement))` over zero triggers is true,
//! and `core/src/precedence.rs` skips such a Binding deliberately, pinned to
//! `shared/oracle.ts` making the identical call. Any signal keyed on "the
//! trigger matched nothing" alone fires on that correct Specification too.
//!
//! So the design is two-level, and only the second level separates a defect from
//! legitimate vacuity:
//!
//! 1. **Per Trace** — [`TriggerExercise`] is a *fact* about one evidence set. It
//!    is reported, never folded into a pass.
//! 2. **Across the validation corpus** — [`CorpusExercise::NeverExercised`] is
//!    the *defect*: a Specification that raised no obligation on **any** Trace it
//!    was validated against has caught nothing, by construction, and ADR-0009
//!    makes adversarial validation a required Promotion gate.
//!
//! # Compile-time vacuity is a different defect, and it is already closed
//!
//! A selector that *provably* cannot match anything is a stronger finding than
//! one that merely matched nothing here, and ADR-0031 §1 prefers the static
//! guarantee. [`SelectorIr`](crate::ir::SelectorIr)'s docs carry the
//! enumeration: this selector language cannot express a contradiction, and blank
//! required-attribute keys are unrepresentable because maps are keyed by
//! [`AttributeKey`](sentinel_model::AttributeKey) (issue #203). Adding
//! negation or ranges to the selector language creates a real static subset, and
//! whoever adds them owns the check for it.
//!
//! # A must-never-match selector is the trap, and the defence is structural
//!
//! A Specification asserting something never happens may legitimately match no
//! Event *by design* — the selector that must never match is the point, while
//! the selector that raises the obligation is elsewhere. Censusing the wrong one
//! would invert the Pattern's meaning: a **clean** deployment would report the
//! Specification as never exercised, which is this module's own defect signal
//! fired at exactly the deployment it should say nothing about.
//!
//! CAT-4 `provenance.taint` is the first catalogued Pattern with that shape, and
//! it is censused on its `action` selector. A Trace full of untrusted-content
//! markers and no action raises no obligation and establishes nothing; a Trace
//! full of actions and no markers raises obligations and is exercised. Both
//! directions are asserted in `compile/tests/instantiation.rs`, because wiring
//! them the wrong way round is plausible rather than absurd.
//!
//! CAT-2 `absence.after-until` is the first absence-shaped Pattern, and it is
//! censused on its `until` selector — never on `veto`. A Trace full of vetoes
//! and no `until` raises no obligation; a Trace that closes intervals cleanly
//! raises one per `until`. `cat2_censuses_its_until_selector_and_never_its_veto`
//! is the forcing test. The defence remains the exhaustive `match` on
//! [`PatternId`](crate::catalogue::PatternId) inside
//! [`PatternInstance::exercise`](crate::catalogue::PatternInstance::exercise),
//! not a shape heuristic.
//!
//! CAT-5 `aggregation.count-within` is the first aggregation-shaped Pattern, and
//! it is censused on its `trigger` (assessment) selector — never on `member`. A
//! Trace full of counted members and no trigger raises no obligation; a Trace
//! that assesses under threshold raises one per trigger.
//! `cat5_censuses_its_trigger_selector_and_never_its_member` is the forcing test.
//!
//! [`CounterpartMatches`] is the same argument applied to the *other* selector's
//! count: what a match there means is opposite across Patterns, so the sense
//! travels in the type rather than in a field name. CAT-2's third sense —
//! [`CounterpartMatches::Interval`] — carries both the opening delimiter count
//! and the falsifying count; Promotion polarity still reads the falsifying half
//! via [`CounterpartMatches::matches`]. CAT-5 reports member matches as
//! [`CounterpartMatches::Falsifying`] (the population a cold over-threshold
//! `Fails` would be drawn from).
//!
//! API listing-cache / Promotion decode grows an `"interval"` arm for
//! [`CounterpartMatches::as_str`] in `api/src/candidate/mapping.rs`. The stored
//! shape holds one counterpart count; encode persists `falsifying` and decode
//! restores `opening` as zero.

#![allow(clippy::doc_markdown)] // Product names and file paths read as prose here.

use std::num::NonZeroUsize;

use sentinel_model::Event;

/// Evidence a caller asserts is complete: no further Event will arrive for it.
///
/// # Why the assertion is in the type
///
/// "The trigger matched nothing" and "the trigger has matched nothing *yet*" are
/// different statements, and only the first is about the Specification. The
/// second is about the clock. Reporting it would be the mirror image of the
/// defect this module exists for — a finding manufactured out of absent
/// evidence.
///
/// The condition that licenses the first reading is the one the Verification
/// Core already gates emission on: the evidence gate is open and Watermarks
/// cover every delivered epoch. `sentinel-compile` cannot see a Frontier, so
/// recomputing that here would be a second source of truth; `core`'s
/// `Evaluator::evidence_is_settled` is the accessor a driver asks instead.
///
/// # This is an assertion, not a proof
///
/// [`SettledEvidence::asserted`] does not verify anything, and ADR-0031 is
/// explicit that sealing a constructor is not the same as establishing an
/// invariant — *"what holds the labels up is one construction site each"*. What
/// the type buys is narrower and still worth having: a caller cannot hand
/// still-arriving evidence to a bare `&[Event]` parameter without writing the
/// word `asserted` at the call site.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct SettledEvidence<'a> {
    events: &'a [Event],
}

impl<'a> SettledEvidence<'a> {
    /// Asserts that `events` is the whole of a settled evidence set.
    #[must_use]
    pub const fn asserted(events: &'a [Event]) -> Self {
        Self { events }
    }

    /// Returns the Events the assertion covers.
    #[must_use]
    pub const fn events(&self) -> &'a [Event] {
        self.events
    }

    /// Returns the denominator: how many Events the assertion covers.
    #[must_use]
    pub const fn examined(&self) -> usize {
        self.events.len()
    }
}

/// Whether a Pattern instance's obligation-raising selector raised anything,
/// and if not, which authoring mistake that points at.
///
/// Five values, not one value with a zero in it. `KindNeverOccurred` and
/// `ValuationNeverMatched` are different mistakes with different remediations —
/// *"you named an Event kind nobody emits"* versus *"you required an attribute
/// valuation nobody sets"* — and `NoEvidenceExamined` is its own answer because
/// *"nothing matched, and I examined 0 Events"* must not render as *"nothing
/// matched, and I examined 122 Events."* The denominator travels with the
/// answer rather than beside it.
///
/// **Deliberately not `#[non_exhaustive]`.** A closed census with no sixth
/// answer is the point: a Promotion gate (#21, #35) that folds this into a
/// decision must be forced to recompile when a value is added, and an
/// exhaustive `match` is the only thing that does that.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum TriggerExercise {
    /// The evidence held no Event at all, so nothing was established.
    ///
    /// Never a pass and never a finding about the Specification: it is a finding
    /// about the validation.
    NoEvidenceExamined,
    /// No Event carried the selector's Event kind.
    ///
    /// The headline case of issue #144: a trigger naming a kind the deployment
    /// never emits.
    KindNeverOccurred {
        /// How many Events were examined to reach that answer.
        events_examined: NonZeroUsize,
    },
    /// The Event kind occurs, but no occurrence satisfied the required
    /// attribute valuation.
    ValuationNeverMatched {
        /// How many Events carried the kind without matching the valuation.
        kind_matches: NonZeroUsize,
    },
    /// Events matched the selector, and none of them names a Binding.
    ///
    /// No Slice exists to hold a Verdict, so no Verdict can be produced. This is
    /// the conjunction, not two independent counts: `Evaluator::step_event`
    /// admits an Event to a Slice only when `binding_of` returns a Binding, and
    /// `precedence::assess` skips a Binding whose trigger set is empty, so an
    /// obligation is raised **iff one Event both matches and binds**. A census
    /// keyed on two separate totals would report obligations for a Specification
    /// whose trigger matches Event *A* while only Event *B* names a Binding, and
    /// the Verification Core emits nothing for that.
    MatchedWithoutBinding {
        /// How many Events matched the selector without naming a Binding.
        matches: NonZeroUsize,
    },
    /// Something was measured.
    RaisedObligations {
        /// How many Events matched the selector.
        matches: NonZeroUsize,
        /// How many of those also named a Binding, and so raised an obligation.
        obligations: NonZeroUsize,
    },
}

impl TriggerExercise {
    /// Returns whether this evidence raised at least one obligation.
    #[must_use]
    pub const fn raised_an_obligation(self) -> bool {
        matches!(self, Self::RaisedObligations { .. })
    }

    /// Returns whether there was any evidence to census.
    ///
    /// The denominator of [`CorpusExercise`]: a Trace that held no Event
    /// establishes nothing either way, so counting it as one a Specification
    /// failed to be exercised on would be the same overstatement this module
    /// exists to refuse.
    #[must_use]
    pub const fn examined_evidence(self) -> bool {
        !matches!(self, Self::NoEvidenceExamined)
    }

    /// Returns a stable identifier for this answer.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoEvidenceExamined => "no-evidence-examined",
            Self::KindNeverOccurred { .. } => "kind-never-occurred",
            Self::ValuationNeverMatched { .. } => "valuation-never-matched",
            Self::MatchedWithoutBinding { .. } => "matched-without-binding",
            Self::RaisedObligations { .. } => "raised-obligations",
        }
    }
}

/// What a Pattern's second selector matched, and which way to read the count.
///
/// Issue #144 shipped this as a bare `requirement_matches: usize` whose doc said
/// a zero "is what an evidence set full of violations looks like". That is true
/// of CAT-1 and the exact inverse of the truth for CAT-4, where a **non**-zero
/// count is the population a Violate would be drawn from. CAT-2 needs a third
/// sense: an interval has both an opening delimiter count and a falsifying
/// count, and collapsing them into one `usize` would lose which is which. A
/// `usize` cannot carry the difference and a reader of a census cannot recover
/// it, so ADR-0031 clause 1 applies: the sense goes in the type, where a caller
/// folding the variants together has to write out which one it is looking at.
///
/// Not `#[non_exhaustive]`, for [`TriggerExercise`]'s reason: a Promotion gate
/// that folds this into a decision must be forced to recompile when a third
/// sense appears — and adding [`CounterpartMatches::Interval`] is that force.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum CounterpartMatches {
    /// Matching Events discharge the obligation the trigger raised.
    ///
    /// Reported rather than judged. A requirement selector matching nothing is
    /// not a defect on its own — it is what an evidence set full of violations
    /// looks like — but it is the first thing to look at when the trigger did
    /// raise obligations and every Verdict came back Violate.
    Discharging(usize),
    /// Matching Events falsify the property where they reach a trigger.
    ///
    /// Reported rather than judged, and read the other way round: zero is what a
    /// clean deployment looks like. A non-zero count is not a finding either —
    /// whether any of these Events actually reaches an action is a causal
    /// question `sentinel-compile` cannot answer, and answering it here would be
    /// the census overstepping into the Verdict's job.
    Falsifying(usize),
    /// An interval-shaped Pattern: opening delimiter matches, plus falsifying
    /// matches inside the open interval.
    ///
    /// [`CounterpartMatches::matches`] returns `falsifying` — that is the
    /// Promotion polarity (a non-zero count is the Violate population), matching
    /// [`CounterpartMatches::Falsifying`]. `opening` travels beside it so a
    /// reader can see whether the interval ever opened without inventing a
    /// second census type.
    Interval {
        /// How many Events matched the opening delimiter (`after`).
        opening: usize,
        /// How many Events matched the prohibited selector (`veto`).
        falsifying: usize,
    },
}

impl CounterpartMatches {
    /// Returns how many Events matched the polarity-bearing selector.
    ///
    /// For [`CounterpartMatches::Interval`] that is `falsifying`, never
    /// `opening`: Promotion asks whether a veto population exists, and treating
    /// the delimiter count as the answer would invert a clean open interval into
    /// a finding.
    #[must_use]
    pub const fn matches(self) -> usize {
        match self {
            Self::Discharging(matches) | Self::Falsifying(matches) => matches,
            Self::Interval { falsifying, .. } => falsifying,
        }
    }

    /// Returns a stable identifier for the sense of the count.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Discharging(_) => "discharging",
            Self::Falsifying(_) => "falsifying",
            Self::Interval { .. } => "interval",
        }
    }
}

/// What one Pattern instance's selectors matched over one settled evidence set.
///
/// A read-side record. Nothing here is encoded on the wire, persisted, or
/// carried on a Verdict — in the case this exists for there is no Verdict and
/// therefore no Assurance Envelope to carry anything.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct SelectorExercise {
    /// Whether the obligation-raising selector raised anything.
    pub trigger: TriggerExercise,
    /// What the Pattern's other selector matched, and in which sense.
    pub counterpart: CounterpartMatches,
    /// How many Events named a Binding, whether or not they matched a selector.
    pub binding_carriers: usize,
    /// The denominator: how many Events were examined.
    pub events_examined: usize,
}

impl SelectorExercise {
    /// Records one Trace's selector census.
    ///
    /// The reconstitution mint: listing-cache rows persist discriminants and
    /// counts, not a live Event slice.
    #[must_use]
    pub const fn recorded(
        trigger: TriggerExercise,
        counterpart: CounterpartMatches,
        binding_carriers: usize,
        events_examined: usize,
    ) -> Self {
        Self {
            trigger,
            counterpart,
            binding_carriers,
            events_examined,
        }
    }
}

/// Whether a Pattern instance was ever exercised across a corpus of evidence.
///
/// The second level of the design in this module's docs, and the only one that
/// separates a defect from legitimate vacuity. A Specification that measured
/// nothing on *one* Trace reported a vacuous truth, which is correct. A
/// Specification that measured nothing on **every** Trace it was validated
/// against caught nothing, and that is what a Promotion gate must refuse.
///
/// Not `#[non_exhaustive]`, for [`TriggerExercise`]'s reason.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum CorpusExercise {
    /// No Trace in the corpus held an Event, so the corpus establishes nothing.
    NothingExamined,
    /// Evidence was examined and no Trace raised an obligation.
    ///
    /// **The defect issue #144 names.**
    NeverExercised {
        /// How many Traces held at least one Event.
        traces: NonZeroUsize,
    },
    /// At least one Trace raised an obligation.
    Exercised {
        /// How many Traces held at least one Event.
        traces: NonZeroUsize,
        /// How many of those raised an obligation.
        exercised_in: NonZeroUsize,
    },
}

impl CorpusExercise {
    /// Folds one census per Trace into the corpus-wide answer.
    #[must_use]
    pub fn of(exercises: impl IntoIterator<Item = TriggerExercise>) -> Self {
        let mut traces = 0usize;
        let mut exercised_in = 0usize;
        for exercise in exercises {
            if !exercise.examined_evidence() {
                continue;
            }
            traces += 1;
            if exercise.raised_an_obligation() {
                exercised_in += 1;
            }
        }
        match (NonZeroUsize::new(traces), NonZeroUsize::new(exercised_in)) {
            (None, _) => Self::NothingExamined,
            (Some(traces), None) => Self::NeverExercised { traces },
            (Some(traces), Some(exercised_in)) => Self::Exercised {
                traces,
                exercised_in,
            },
        }
    }

    /// Returns whether the corpus exercised this Pattern instance at all.
    ///
    /// `false` for [`CorpusExercise::NothingExamined`] as well as for
    /// [`CorpusExercise::NeverExercised`]: a corpus that examined nothing is not
    /// evidence that a Specification works, which is the whole invariant —
    /// missing evidence is never a pass.
    #[must_use]
    pub const fn was_exercised(self) -> bool {
        matches!(self, Self::Exercised { .. })
    }
}

/// Builds the census for one obligation-raising selector over settled evidence.
///
/// Kept here rather than in `instance.rs` so the derivation of the five values
/// is written once, next to the type that names them. The caller supplies the
/// counts because which selector raises the obligation is a per-Pattern
/// question — see this module's docs on absence shapes.
pub(crate) fn classify(
    events_examined: usize,
    kind_matches: usize,
    matches: usize,
    obligations: usize,
) -> TriggerExercise {
    match (
        NonZeroUsize::new(events_examined),
        NonZeroUsize::new(kind_matches),
        NonZeroUsize::new(matches),
        NonZeroUsize::new(obligations),
    ) {
        (None, _, _, _) => TriggerExercise::NoEvidenceExamined,
        (Some(events_examined), None, _, _) => {
            TriggerExercise::KindNeverOccurred { events_examined }
        }
        (Some(_), Some(kind_matches), None, _) => {
            TriggerExercise::ValuationNeverMatched { kind_matches }
        }
        (Some(_), Some(_), Some(matches), None) => {
            TriggerExercise::MatchedWithoutBinding { matches }
        }
        (Some(_), Some(_), Some(matches), Some(obligations)) => {
            TriggerExercise::RaisedObligations {
                matches,
                obligations,
            }
        }
    }
}
