//! **P4, P6 and P8 are three instantiations of one Pattern.** Proven, not claimed.
//!
//! The implementation plan for issue #14 mapped SagaShop's frozen oracle
//! properties onto proposed Patterns in a table. A table is a claim. This file
//! turns three of its rows into assertions: each of the three hand-written
//! `PrecedenceSpecification` fixtures in `core/tests/support/mod.rs` is rebuilt
//! by binding CAT-1 `precedence.approval`'s parameters, lowering the resulting
//! monitor IR, and compared for **structural equality** with `==`. If the
//! catalogue's grammar could not express the testbed's properties, or expressed
//! them differently, these tests fail.
//!
//! That is the strongest artifact in #14a, and it is the one the adversarial
//! review left untouched: every blocker it found lives in CAT-2's decision
//! table, in O-03 retirement, or in the `MonitoredSpecification` refactor's
//! acceptance gate, and none of those is in this scope.
//!
//! Equality alone would not be enough — two Specifications can be equal and both
//! wrong — so the catalogue-built specs are then run through the same two Tier A
//! loops the hand-written ones already pass: the frozen property matrix over all
//! three vendored captures, and 36 arrival-order permutations.
//!
//! `e2e_oracle.rs`, `tracer_bullet.rs`, `resumable.rs`, `corpus_semantic_gate.rs`
//! and `testdata/**` are the frozen baseline and are not modified. The loops
//! below are re-runs over new inputs, not edits to the guarantee.

#![allow(clippy::doc_markdown)] // Product names and file paths read as prose here.

mod support;

use std::num::NonZeroUsize;

use sentinel_compile::catalogue::{
    declaration, CorpusExercise, CounterpartMatches, InstanceWarning, PatternId, PatternInstance,
    SelectorExercise, SettledEvidence, TriggerExercise,
};
use sentinel_compile::{aggregate_by_pattern, compile_filter, FilterSource};
use sentinel_core::evaluator::{Evaluator, Input};
use sentinel_core::specification::PrecedenceSpecification;
use sentinel_core::verdict::BindingVerdict;
use sentinel_ingest::evidence::ObservedEvidence;
use sentinel_model::{AttributeKey, FilterEpoch, IdentifierError, SourceTier, VerdictOutcome};

use support::{
    adapted, all_cat1_instances, all_precedence_specifications, arrival_orders,
    cat1_charge_requires_prior_reserve, cat1_fulfill_requires_prior_charge,
    cat1_reserve_requires_prior_create, cat1_ship_requires_prior_charge,
    cat1_ship_requires_prior_charge_bound_to, cat1_ship_requires_prior_charge_triggered_by,
    charge_requires_prior_reserve, encoded_verdicts, evaluate, frozen_property_expectation,
    fulfill_requires_prior_charge, reordered, ship_requires_prior_charge, ORDER_SERVICE,
    SAGASHOP_BINDING_ATTRIBUTE,
};

/// Lowers a catalogue instance into the Core's Specification.
fn lowered(instance: &PatternInstance) -> PrecedenceSpecification {
    PrecedenceSpecification::try_from(
        instance
            .ir()
            .as_precedence()
            .expect("CAT-1 lowers to the precedence monitor"),
    )
    .expect("catalogued CAT-1 IR converts")
}

/// Lowers a catalogue instance's census over one scenario's evidence.
///
/// `SettledEvidence::asserted` is an assertion, and here is what discharges it:
/// a vendored capture is a recorded, finished Trace. Nothing further will arrive
/// for it, ever. That is the whole reason ADR-0013 makes the engine
/// trace-driven, and it is why these fixtures can answer a question a live
/// deployment could not.
fn censused(instance: &PatternInstance, evidence: &ObservedEvidence) -> SelectorExercise {
    instance.exercise(SettledEvidence::asserted(evidence.events()))
}

/// The aggregation `e2e_oracle.rs` applies to a precedence property's scenario
/// cell, with every way of passing without measuring anything separated out
/// instead of folded into `true`.
///
/// This used to be `verdicts.iter().all(|b| b.outcome() == Satisfy)` returning a
/// bare `bool`, and that shape cannot distinguish "every Binding satisfied its
/// obligation" from "nothing was measured": `Iterator::all` over an empty slice
/// is `true`. The distinction is not academic here, and it turns out to have two
/// quite different instances that the one boolean collapsed together.
///
/// **The defect.** A `PrecedenceSpecification` groups Events by the *value* its
/// Binding attribute holds, and `binding_of` returns `None` for every Event
/// whose producer attributes do not carry that key. A Binding attribute of `""`,
/// of `"   "`, or of `"sagashop.order.i"` — one character short of the real key —
/// therefore yields zero Bindings, zero Verdicts, no error and no warning. All
/// nine (scenario, property) cells this file checks expect `"pass"`, so the old
/// boolean reported nine greens for a Specification that had bound nothing at
/// all. `instantiate` now refuses the first two by value; it cannot refuse the
/// third, because `sagashop.order.i` is a well-formed attribute key that no
/// check inside `sentinel-compile` could tell from a real key whose producer has
/// not shipped yet. Catching it needs the evidence, so it has to happen here.
/// This matters because #21 (generation) and #35 (the promotion form) both feed
/// unauthored strings into `ParameterKind::AttributeKey`, and a Specification
/// reporting Satisfy over zero Bindings is ADR-0017's silent pass wearing an
/// Assurance Envelope.
///
/// **The legitimate case, which is live in this repository right now.** Bindings
/// can exist and still produce no Verdict, because no Event raised the
/// obligation. `cancel.P4` and `cancel.P8` are exactly that: the cancel capture
/// cancels its one order before anything ships or is fulfilled, so P4's and P8's
/// trigger selectors match nothing and the frozen matrix's `"pass"` for those
/// two cells means *"no ship was ever requested"*, not *"every ship had a prior
/// charge"*. That is a true reading of a precedence obligation and must keep
/// passing. It is also invisible in a boolean, which is why it gets its own
/// variant and why `the_frozen_matrix_cells_with_no_obligation_are_named` pins
/// which cells are in it.
///
/// **The discriminator is the trigger census, not the Binding count** (issue
/// #144). The Binding count separates "nothing to check" from "bound nothing",
/// which is the axis #14a closed — and it is exactly the quantity a wrong
/// *trigger* does not move. A one-character typo in P4's trigger kind leaves
/// `bound = 16 / 14 / 60` on the three captures and drops the Verdicts to zero,
/// so `NoObligation` fires on all nine cells and the primary Verdict check
/// reads green for a wholly non-functional Specification. Both discriminators
/// are needed and both are here: `sentinel_compile`'s `SelectorExercise`
/// carries the trigger census *and* the Binding-carrier count, derived in one
/// pass by the same matcher the Verification Core evaluates with.
///
/// The census cannot make a per-cell judgement out of `NoObligation`, and must
/// not try. See `catalogue_built_specs_match_frozen_matrix` for the fold that
/// can.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PropertyOutcome {
    /// At least one Verdict, and every one settled Satisfy.
    AllSatisfy,
    /// At least one Verdict, and at least one did not settle Satisfy.
    NotAllSatisfy,
    /// No trigger Event occurred, so no obligation was raised.
    ///
    /// Named for its cause now rather than for the observation. `cancel.P4` and
    /// `cancel.P8` are this, and `"pass"` for them is the vacuous truth of an
    /// obligation never raised. **On its own evidence this is correct and must
    /// keep passing**; whether the Specification is defective is a question
    /// about its whole validation corpus, which
    /// [`CorpusExercise`] answers and this value deliberately does not.
    NoObligation,
    /// No Slice exists to hold a Verdict: nothing that matters named a Binding.
    ///
    /// Covers both shapes of that, because both mean the same thing to a
    /// Verdict — the Specification's Binding attribute is carried by nothing in
    /// the evidence, or trigger Events occurred and none of them named a
    /// Binding. Never a pass, whatever the matrix expects.
    NoBindings,
    /// Obligations were raised and no Verdict came back.
    ///
    /// The Evaluator's emission gate can withhold one, which is the honest limit
    /// #14a's doc comment named and this file previously could not distinguish.
    /// No SagaShop cell reaches it today; no claim is made here that none can,
    /// and if one ever does it is not a pass.
    ObligationWithheld,
    /// The evidence held no Event, so the cell established nothing.
    ///
    /// "Nothing matched, and I examined 0 Events" is a finding about the
    /// validation, not about the Specification. No vendored capture is empty
    /// today; the value exists so an empty one could never read as green.
    NoEvidenceExamined,
}

impl PropertyOutcome {
    /// Returns whether this outcome is the frozen matrix's `"pass"`.
    ///
    /// An exhaustive `match` rather than a `matches!`, so a value added to the
    /// census is a compile error here rather than a silent not-a-pass. The four
    /// non-passes are separated so the failure message says *why*.
    const fn is_pass(self) -> bool {
        match self {
            Self::AllSatisfy | Self::NoObligation => true,
            Self::NotAllSatisfy
            | Self::NoBindings
            | Self::ObligationWithheld
            | Self::NoEvidenceExamined => false,
        }
    }
}

/// Aggregates one property's Verdicts over one scenario's evidence.
///
/// Takes the census rather than the Specification and the evidence: what
/// separates "nothing to check" from "checked nothing" is not recoverable from
/// the Verdict vector, and keying it on the Binding count alone is issue #144's
/// defect. The `match` is exhaustive over [`TriggerExercise`].
fn aggregate(exercise: SelectorExercise, verdicts: &[BindingVerdict]) -> PropertyOutcome {
    if !verdicts.is_empty() {
        return if verdicts
            .iter()
            .all(|binding| binding.verdict().outcome() == VerdictOutcome::Satisfy)
        {
            PropertyOutcome::AllSatisfy
        } else {
            PropertyOutcome::NotAllSatisfy
        };
    }
    match exercise.trigger {
        TriggerExercise::NoEvidenceExamined => PropertyOutcome::NoEvidenceExamined,
        TriggerExercise::MatchedWithoutBinding { .. } => PropertyOutcome::NoBindings,
        // CAT-2 `#144` contract: `until` matched so the census reports
        // RaisedObligations, but every until was NeverOpened+settled and the
        // Core emitted no Verdict (NoObligation). CounterpartMatches::Interval
        // is what distinguishes that honest empty vector from a withheld one.
        TriggerExercise::RaisedObligations { .. } => match exercise.counterpart {
            CounterpartMatches::Interval { .. } => PropertyOutcome::NoObligation,
            CounterpartMatches::Discharging(_) | CounterpartMatches::Falsifying(_) => {
                PropertyOutcome::ObligationWithheld
            }
        },
        TriggerExercise::KindNeverOccurred { .. }
        | TriggerExercise::ValuationNeverMatched { .. } => {
            // #14a's axis, kept: a Specification bound to a key nobody emits
            // raises no obligation *and* holds no Slice, and the second is the
            // stronger statement. Without this the mis-bound instance would
            // read as an obligation that legitimately never arose on the one
            // capture where its trigger also matches nothing.
            if exercise.binding_carriers == 0 {
                PropertyOutcome::NoBindings
            } else {
                PropertyOutcome::NoObligation
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The proof
// ---------------------------------------------------------------------------

/// **P4, P6 and P8 are three instantiations of CAT-1.**
///
/// Structural equality against the hand-written fixtures, which
/// `PrecedenceSpecification` supports because it derives `PartialEq`. Every
/// field is compared: identity, Binding attribute, both selectors with their
/// required attribute valuations, the required-source set with each source's
/// declared minimum tier, the schema version, and the cold-guard set.
///
/// The width fan-out `compile/src/width.rs` asked #14 to aggregate is exactly
/// this: one template, three Specifications.
#[test]
fn p4_p6_p8_are_cat1_instantiations() {
    let cases = [
        (
            "P4",
            cat1_ship_requires_prior_charge(),
            ship_requires_prior_charge(),
        ),
        (
            "P6",
            cat1_charge_requires_prior_reserve(),
            charge_requires_prior_reserve(),
        ),
        (
            "P8",
            cat1_fulfill_requires_prior_charge(),
            fulfill_requires_prior_charge(),
        ),
    ];

    for (property, instance, hand_written) in cases {
        assert_eq!(instance.pattern(), PatternId::PrecedenceApproval);
        assert_eq!(
            lowered(&instance),
            hand_written,
            "{property}: the catalogue's instantiation is not the hand-written Specification"
        );
    }
}

/// The three instances come from one template, and nothing about the template
/// varies between them.
///
/// This is the half of the claim equality does not cover: three equal
/// Specifications built by three unrelated constructors would also pass the test
/// above. What makes them *instantiations* is that only the arguments differ.
#[test]
fn the_three_instantiations_share_one_template() {
    let declared = declaration(PatternId::PrecedenceApproval);
    for (property, instance) in all_cat1_instances() {
        assert_eq!(
            instance.pattern(),
            PatternId::PrecedenceApproval,
            "{property}"
        );
        assert_eq!(instance.declaration().id, declared.id, "{property}");
        assert_eq!(instance.width_budget(), declared.width.budget, "{property}");
        assert_eq!(instance.declaration().cost, declared.cost, "{property}");
    }
}

// ---------------------------------------------------------------------------
// The same bars the hand-written Specifications already clear
// ---------------------------------------------------------------------------

/// Catalogue-built Specifications match SagaShop's frozen oracle matrix — and
/// every one of them measured something somewhere.
///
/// Same properties, same three vendored captures, same aggregation as
/// `e2e_oracle.rs`'s Tier A fidelity test, over Specifications produced by the
/// Pattern catalogue. `testdata/` is untouched: the expectations are read from
/// the frozen matrix, not restated here.
///
/// **The corpus fold is the part issue #144 adds, and it is what stops this
/// test from being the one the issue was filed about.** It reported: *"the
/// primary Verdict check passes on a wholly non-functional Specification,
/// because `NoObligation` is a pass and the typo makes every cell
/// `NoObligation`."* Nine green cells, no error, no warning. Per cell that
/// cannot be fixed — a cell where the obligation never arose is a true pass and
/// `cancel.P4` is one — so the guard is corpus-wide: a Specification that raised
/// no obligation on **any** of the three captures measured nothing at all, and
/// `CorpusExercise::of` says so.
///
/// The loop is property-outer for that reason: the fold is over captures, so the
/// captures have to be the inner dimension.
#[test]
fn catalogue_built_specs_match_frozen_matrix() {
    let scenarios: Vec<(&str, ObservedEvidence)> = ["happy", "cancel", "hazards"]
        .into_iter()
        .map(|scenario| (scenario, adapted(scenario)))
        .collect();

    for (property, instance) in all_cat1_instances() {
        let specification = lowered(&instance);
        let mut corpus = Vec::new();

        for (scenario, evidence) in &scenarios {
            let verdicts = evaluate(&specification, evidence).expect("evaluation succeeds");
            let exercise = censused(&instance, evidence);
            corpus.push(exercise.trigger);

            let outcome = aggregate(exercise, &verdicts);
            assert_ne!(
                outcome,
                PropertyOutcome::NoBindings,
                "{scenario}.{property}: no Slice existed to hold a Verdict, so this cell would \
                 have read green without measuring anything"
            );
            assert_eq!(
                outcome.is_pass(),
                frozen_property_expectation(scenario, property) == "pass",
                "{scenario}.{property}: catalogue-built spec disagrees with the frozen matrix"
            );
        }

        assert!(
            CorpusExercise::of(corpus).was_exercised(),
            "{property}: this Specification raised no obligation on any of the three captures, \
             so every cell above is a vacuous truth and the whole row read green without \
             measuring anything"
        );
    }
}

/// Catalogue-built Specifications produce byte-identical Verdicts across 36
/// arrival orders, for all three properties and all three scenarios.
///
/// Worth being precise about what this does and does not prove, because
/// `core/src/precedence.rs` is precise about it: arrival-order permutations are
/// not admissible linearizations, so this is an implementation-determinism test
/// and does not discharge ADR-0020's trace-closure obligation. What it catches
/// is a lowering that let iteration order or accumulation order into the Verdict
/// — which is a real risk here, since the lowering rebuilds a `BTreeSet` of
/// required sources out of a `BTreeMap`.
#[test]
fn catalogue_built_specs_are_arrival_order_invariant() {
    let orders = arrival_orders();
    for scenario in ["happy", "cancel", "hazards"] {
        let evidence = adapted(scenario);
        for (property, instance) in all_cat1_instances() {
            let specification = lowered(&instance);
            let baseline =
                encoded_verdicts(&evaluate(&specification, &evidence).expect("baseline evaluates"));

            // Without this, an empty baseline makes every comparison below
            // `[] == []` — vacuously true across all 36 orders. `cancel.P4` and
            // `cancel.P8` legitimately produce no Verdict, so two of nine cells
            // would assert nothing and the loop would still report success. That
            // is the same defect this file's `PropertyOutcome` exists to close,
            // one level down, and it was demonstrated rather than supposed: under
            // a perturbation that made every cell bind nothing, this was the only
            // one of six catalogue-grounding loops that stayed green.
            //
            // Cells with no obligation are skipped rather than asserted on,
            // because there is nothing to hold invariant when nothing was emitted
            // — and `the_frozen_matrix_cells_with_no_obligation_are_named` is what
            // stops that skip from silently widening.
            if baseline.is_empty() {
                assert!(
                    matches!(
                        aggregate(censused(&instance, &evidence), &[]),
                        PropertyOutcome::NoObligation
                    ),
                    "{scenario} {property} produced no Verdict for a reason other \
                     than having no obligation, so arrival-order invariance is \
                     being skipped for a cell that should have emitted something"
                );
                continue;
            }

            for (index, order) in orders.iter().enumerate() {
                let permuted = reordered(&evidence, order);
                let verdicts = evaluate(&specification, &permuted).expect("permuted evaluates");
                assert_eq!(
                    encoded_verdicts(&verdicts),
                    baseline,
                    "{scenario} {property} arrival order {index} changed the Verdict"
                );
            }
        }
    }
}

/// A catalogue-built Specification and its hand-written twin produce identical
/// Verdict bytes on the same evidence.
///
/// Stronger than the two loops above and cheap to state: the frozen-matrix loop
/// compares an aggregate boolean, so it could not distinguish `Violate` from
/// `Inconclusive`. This compares the encoded Verdicts.
#[test]
fn catalogue_built_and_hand_written_verdicts_are_byte_identical() {
    let hand_written = all_precedence_specifications();
    for scenario in ["happy", "cancel", "hazards"] {
        let evidence = adapted(scenario);
        for ((property, instance), (twin_property, twin)) in
            all_cat1_instances().into_iter().zip(hand_written.iter())
        {
            assert_eq!(property, *twin_property, "fixture ordering drifted");
            let from_catalogue =
                encoded_verdicts(&evaluate(&lowered(&instance), &evidence).expect("evaluates"));
            let from_fixture = encoded_verdicts(&evaluate(twin, &evidence).expect("evaluates"));
            assert_eq!(
                from_catalogue, from_fixture,
                "{scenario}.{property}: catalogue-built and hand-written Verdicts differ"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The vacuous instantiation
// ---------------------------------------------------------------------------

/// **A Binding attribute nobody emits produces no Verdicts, and that is now a
/// failure rather than a pass.**
///
/// This is the composed silent pass, pinned as detectable. `sagashop.order.i` is
/// `sagashop.order.id` with one character missing — the shape of typo #21's
/// generator or #35's form will eventually produce. It is a well-formed
/// attribute key, so `instantiate` returns `Ok`; `validate()` returns `[]`,
/// because a tier and capability check has nothing to say about it; the lowering
/// is clean; evaluation returns `Ok`. What comes back is an empty Verdict
/// vector, on all three vendored captures.
///
/// The test asserts three things in the order that makes the argument:
///
/// 1. the mis-bound Specification really does evaluate without error, so this is
///    a silent pass and not a crash somebody would have noticed;
/// 2. the *old* aggregation rule — `Iterator::all` over that empty vector —
///    returns `true`, which is what made `catalogue_built_specs_match_frozen_matrix`
///    read green across all nine cells;
/// 3. [`aggregate`] classifies it [`PropertyOutcome::NoBindings`], which is what
///    the frozen-matrix loop now refuses.
///
/// Point 2 is deliberately a live assertion rather than a comment. If someone
/// later changes the evaluator so that a Specification binding nothing returns
/// something other than an empty vector, this test tells them the reasoning
/// behind the guard has moved.
#[test]
fn a_typo_in_the_binding_attribute_is_a_detectable_vacuous_instance() {
    let mis_bound = cat1_ship_requires_prior_charge_bound_to(
        AttributeKey::new("sagashop.order.i").expect("well-formed but unused key"),
    )
    .expect("a well-formed attribute key instantiates, whether or not anything emits it");
    assert!(
        mis_bound.validate().is_empty(),
        "validate() has nothing to say about a Binding attribute: it compares tiers and \
         capabilities, not values"
    );

    let specification = lowered(&mis_bound);
    for scenario in ["happy", "cancel", "hazards"] {
        let evidence = adapted(scenario);
        let verdicts = evaluate(&specification, &evidence).expect("evaluation succeeds");

        assert!(
            verdicts.is_empty(),
            "{scenario}: the mis-bound Specification produced Verdicts, so this test no longer \
             constructs the vacuous case"
        );
        assert!(
            verdicts
                .iter()
                .all(|binding| binding.verdict().outcome() == VerdictOutcome::Satisfy),
            "{scenario}: the aggregation this guard replaced returned false, so the silent pass \
             it was written for is no longer reachable this way"
        );
        assert_eq!(
            aggregate(censused(&mis_bound, &evidence), &verdicts),
            PropertyOutcome::NoBindings,
            "{scenario}: a Specification with no Slice to hold a Verdict was not classified \
             as such"
        );
    }
}

/// The same evidence, the same Pattern, the correct Binding attribute: every
/// scenario binds something.
///
/// The control for the test above, and the assertion that makes
/// `catalogue_built_specs_match_frozen_matrix`'s `NoBindings` guard meaningful
/// rather than trivially satisfiable. Without it, "no cell is `NoBindings`" would
/// be equally consistent with the fixtures having quietly stopped loading.
#[test]
fn every_catalogue_built_specification_binds_something_on_every_scenario() {
    for scenario in ["happy", "cancel", "hazards"] {
        let evidence = adapted(scenario);
        for (property, instance) in all_cat1_instances() {
            let specification = lowered(&instance);
            let bound = evidence
                .events()
                .iter()
                .filter_map(|event| specification.binding_of(event))
                .count();
            assert!(
                bound > 0,
                "{scenario}.{property}: the Specification's Binding attribute is carried by no \
                 Event in this capture"
            );
        }
    }
}

/// **Two of the nine frozen-matrix cells pass because nothing triggered the
/// obligation, and this is the list.**
///
/// `cancel.P4` and `cancel.P8` produce Bindings and zero Verdicts: the cancel
/// capture cancels its one order before it ships or is fulfilled, so neither
/// property's trigger selector matches anything. `"pass"` for those two cells is
/// therefore the vacuous truth of an obligation never raised, which is correct —
/// and completely invisible in the boolean the matrix loop used to compare.
///
/// Pinning the list is the point. A change that emptied a *third* cell would
/// otherwise be indistinguishable from the frozen matrix continuing to agree,
/// and "the suite recorded that the fixes were written, not that they held"
/// (`store/tests/postgres_regression_coverage.rs`) is this repository's own name
/// for that failure. The same two cells are aggregated the same way by
/// `core/tests/e2e_oracle.rs`, which is frozen and not edited here.
#[test]
fn the_frozen_matrix_cells_with_no_obligation_are_named() {
    let expected_vacuous = [("cancel", "P4"), ("cancel", "P8")];

    let mut observed = Vec::new();
    for scenario in ["happy", "cancel", "hazards"] {
        let evidence = adapted(scenario);
        for (property, instance) in all_cat1_instances() {
            let specification = lowered(&instance);
            let verdicts = evaluate(&specification, &evidence).expect("evaluation succeeds");
            if aggregate(censused(&instance, &evidence), &verdicts) == PropertyOutcome::NoObligation
            {
                observed.push((scenario, property));
            }
        }
    }

    assert_eq!(
        observed,
        expected_vacuous.to_vec(),
        "the set of frozen-matrix cells that pass without raising an obligation changed"
    );
}

/// **An empty or whitespace-only Binding attribute never reaches evaluation.**
///
/// Issue #203 closed the vacuous-pass shape that used to reach
/// `InstanceError::BlankAttributeKey`. Blank names are refused by
/// [`AttributeKey::new`] before a [`ParameterValue`] or Specification exists;
/// those InstanceError arms are gone. Identity blanks still use
/// [`InstanceError::BlankIdentity`] because Pattern IR identity fields stay
/// public `String`.
///
/// Note what is being asserted about `SAGASHOP_BINDING_ATTRIBUTE`: the same
/// key still instantiates, so the check rejects blanks and not attribute keys
/// in general.
#[test]
fn a_blank_binding_attribute_is_refused_before_a_specification_exists() {
    for blank in ["", " ", "   ", "\t", "\n", " \t\n "] {
        assert_eq!(
            AttributeKey::new(blank),
            Err(IdentifierError),
            "{blank:?} must be refused by AttributeKey::new"
        );
    }

    cat1_ship_requires_prior_charge_bound_to(
        AttributeKey::new(SAGASHOP_BINDING_ATTRIBUTE).expect("fixture binding"),
    )
    .expect("the real Binding attribute is still accepted");
}

// ---------------------------------------------------------------------------
// P7: the tier shortfall
// ---------------------------------------------------------------------------

/// **P7 surfaces a tier shortfall as a warning, and instantiates anyway.**
///
/// `core/tests/e2e_oracle.rs` records this in English: "P7 is an oracle check
/// because its requirement (`order.create.requested`) comes from a
/// Contributing-tier source, so the Core's anchor rule could never produce an
/// authoritative verdict for it." That caveat has lived in a test comment, where
/// nothing reads it and nothing checks it.
///
/// Here it is mechanical. CAT-1 declares that `requirement_source` needs anchor
/// tier; the Adapter places `sagashop-order` at contributing tier because it is
/// inside the Monitored Application; `validate()` reports the gap.
///
/// It is a **warning and not an error**, and that is ADR-0022 rather than
/// leniency: the required tier is a Specification property, and where a source
/// actually sits is a deployment fact. Refusing to instantiate would be the
/// compiler overruling a deployment it cannot see. Moving `sagashop-order`
/// behind an anchor-tier probe would make this same Specification authoritative
/// with no change to the Pattern.
#[test]
fn p7_reports_a_tier_shortfall_and_still_instantiates() {
    let instance = cat1_reserve_requires_prior_create();

    let warnings = instance.validate();
    assert_eq!(
        warnings.len(),
        1,
        "expected exactly the tier shortfall, got {warnings:?}"
    );
    match &warnings[0] {
        InstanceWarning::TierShortfall {
            role,
            source,
            required,
            placement,
        } => {
            assert_eq!(*role, "requirement_source");
            assert_eq!(source.monitored_application().as_str(), ORDER_SERVICE);
            assert_eq!(*required, SourceTier::Anchor);
            assert_eq!(*placement, SourceTier::Contributing);
        }
        other => panic!("expected a TierShortfall, got {other:?}"),
    }
}

/// P7's lowered Specification keeps demanding anchor tier.
///
/// The demand must not shrink to match the deployment. A Specification that
/// lowered its own requirement to whatever the deployment happened to supply
/// could never report that the deployment is insufficient — which is precisely
/// the silent pass ADR-0017 exists to prevent, moved one layer earlier.
#[test]
fn p7_keeps_demanding_an_anchor_the_deployment_does_not_place() {
    let specification = lowered(&cat1_reserve_requires_prior_create());
    let order = support::source(ORDER_SERVICE);
    assert_eq!(
        specification.minimum_tier_of(&order),
        Some(SourceTier::Anchor),
        "the Specification's demand was weakened to match the deployment"
    );
}

// ---------------------------------------------------------------------------
// Declared versus measured
// ---------------------------------------------------------------------------

/// CAT-1's declared Cost Class survives contact with the evaluator.
///
/// `core/src/cost.rs` supplies the measured actual and its module doc assigns
/// the declaration to #14/#15; this is the first place the two meet. CAT-1
/// declares hot with zero expected fact requests per Verdict, so the lowered
/// Specification must declare no cold guard — a Specification that did would
/// make the batch path return `CoreError::FactsRequired` rather than a Verdict,
/// which is exactly how a wrongly declared Cost Class would show up.
#[test]
fn declared_cost_class_matches_the_evaluated_cost() {
    let declared = declaration(PatternId::PrecedenceApproval).cost;
    assert_eq!(declared.expected_fact_requests_per_verdict, 0);

    let evidence = adapted("happy");
    for (property, instance) in all_cat1_instances() {
        let specification = lowered(&instance);
        assert!(
            specification.cold_guards().is_empty(),
            "{property}: a hot Pattern lowered to a Specification with cold guards"
        );
        evaluate(&specification, &evidence)
            .unwrap_or_else(|error| panic!("{property}: hot evaluation failed: {error}"));
    }
}

// ---------------------------------------------------------------------------
// Per-Pattern width aggregation
// ---------------------------------------------------------------------------

/// **One template, three Specifications, one aggregated width report.**
///
/// `compile/src/width.rs` deferred this to #14: "per-Pattern aggregation across
/// multiple instances of one Pattern template ... is deferred to issue #14,
/// which owns the Pattern catalogue and therefore the template-to-instance
/// fan-out". P4, P6 and P8 *are* that fan-out, so this is the deferral closed on
/// the real case rather than on a constructed one.
///
/// It also supplies the observation behind CAT-1's `measured: true`. Every
/// instance emits two Event kinds and two attribute keys, actual width 4 against
/// a declared 6 — the one width in the catalogue that has been observed rather
/// than estimated. Sum and max are both reported because they answer different
/// questions: sum is what the deployment pays for shipping Events, max is what
/// the Pattern author is answerable for, and one sprawling instance and ten tidy
/// ones sum alike.
#[test]
fn aggregate_by_pattern_sums_three_cat1_instances() {
    let instances: Vec<_> = all_cat1_instances()
        .into_iter()
        .map(|(_, instance)| instance)
        .collect();
    let sources: Vec<&dyn FilterSource> = instances
        .iter()
        .map(|instance| instance as &dyn FilterSource)
        .collect();
    let compiled = compile_filter(&sources, FilterEpoch::new(14)).expect("three instances compile");

    let attributed: Vec<_> = instances
        .iter()
        .map(PatternInstance::pattern)
        .zip(compiled.widths.iter().cloned())
        .collect();
    let aggregated = aggregate_by_pattern(&attributed);

    assert_eq!(aggregated.len(), 1, "three instances, one template");
    let report = &aggregated[&PatternId::PrecedenceApproval];
    assert_eq!(report.instances, 3);

    let declared = declaration(PatternId::PrecedenceApproval).width;
    let declared_width = declared.budget.event_kinds + declared.budget.attribute_keys;
    assert_eq!(report.declared_width, Some(declared_width));

    // Two kinds and two keys per instance: the observation behind `measured`.
    assert!(declared.measured, "CAT-1's budget is the measured one");
    for width in &compiled.widths {
        assert_eq!(width.actual_event_kinds, 2);
        assert_eq!(width.actual_attribute_keys, 2);
        assert_eq!(width.actual_width, 4);
    }
    assert_eq!(report.max_actual_width, 4);
    assert_eq!(report.total_actual_width, 12);
    assert_eq!(
        report.instances_over_budget, 0,
        "CAT-1's declared budget holds for every instance of CAT-1"
    );
}

// ---------------------------------------------------------------------------
// The vacuous trigger (issue #144)
// ---------------------------------------------------------------------------

/// **A trigger selector nobody emits produces no Verdicts, and that is now
/// detectable.**
///
/// The acceptance criterion of issue #144, and the mirror of
/// `a_typo_in_the_binding_attribute_is_a_detectable_vacuous_instance` line for
/// line. `sagashop.order.ship.requeste` is P4's real trigger with one character
/// missing — the shape a generated Candidate produces when its Intent Artifact
/// names a tool the deployment does not call.
///
/// The asymmetry the issue reported: a wrong Binding attribute moves the Binding
/// count, and a wrong *trigger* does not. `bound` stays at 16 / 14 / 60 on the
/// three captures while the Verdicts drop to zero, so every discriminator keyed
/// on the Binding count reports "an obligation that never arose" — a pass.
///
/// Five assertions, in the order that makes the argument:
///
/// 1. `instantiate` returns `Ok`, so this is a silent pass and not a crash
///    somebody would have noticed;
/// 2. `validate()` is still `[]`, the honest statement of the gap, kept live;
/// 3. evaluation returns `Ok` with zero Verdicts on all three captures, and the
///    *old* `Iterator::all` over that empty vector returns `true` — which is
///    what made the primary Verdict check read green;
/// 4. the census names the cause on every capture, and `validate_against`
///    reports it as a warning;
/// 5. the corpus fold is `NeverExercised`, which is the value that separates
///    this from `cancel.P4` and the one the frozen-matrix loop now refuses.
#[test]
fn a_typo_in_the_trigger_selector_is_a_detectable_vacuous_instance() {
    let mistyped = cat1_ship_requires_prior_charge_triggered_by("sagashop.order.ship.requeste")
        .expect("a well-formed Event kind instantiates, whether or not anything emits it");
    assert!(
        mistyped.validate().is_empty(),
        "validate() compares tiers and capabilities: it cannot see that a kind is never emitted"
    );

    let specification = lowered(&mistyped);
    let mut corpus = Vec::new();

    for scenario in ["happy", "cancel", "hazards"] {
        let evidence = adapted(scenario);
        let verdicts = evaluate(&specification, &evidence).expect("evaluation succeeds");

        assert!(
            verdicts.is_empty(),
            "{scenario}: the mistyped Specification produced Verdicts, so this test no longer \
             constructs the vacuous case"
        );
        assert!(
            verdicts
                .iter()
                .all(|binding| binding.verdict().outcome() == VerdictOutcome::Satisfy),
            "{scenario}: the aggregation this guard replaced returned false, so the silent pass \
             it was written for is no longer reachable this way"
        );

        let exercise = censused(&mistyped, &evidence);
        assert!(
            exercise.binding_carriers > 0,
            "{scenario}: this Specification still binds, which is exactly why the Binding count \
             cannot detect it"
        );
        assert!(
            matches!(exercise.trigger, TriggerExercise::KindNeverOccurred { .. }),
            "{scenario}: expected the trigger kind to be reported as never occurring, got {:?}",
            exercise.trigger
        );
        assert!(
            mistyped
                .validate_against(SettledEvidence::asserted(evidence.events()))
                .iter()
                .any(|warning| matches!(warning, InstanceWarning::TriggerKindNeverOccurred { .. })),
            "{scenario}: validate_against did not report the unexercised trigger"
        );
        corpus.push(exercise.trigger);
    }

    assert_eq!(
        CorpusExercise::of(corpus),
        CorpusExercise::NeverExercised {
            traces: NonZeroUsize::new(3).expect("three captures")
        },
        "a Specification exercised by no capture in its corpus was not reported as such"
    );
}

/// **The Verdicts cannot tell a vacuous cell from a broken Specification, and
/// the census can.**
///
/// Same capture, same Pattern, two instances: `cancel.P4` with its real trigger,
/// which measures nothing because the order was cancelled before it shipped, and
/// P4 with a mistyped trigger, which measures nothing because it can never
/// measure anything. Both produce byte-identical (empty) Verdict vectors. The
/// aggregation is the same `NoObligation` for both, correctly — on this evidence
/// they *are* the same fact.
///
/// What differs is the corpus, and only the corpus. This is the assertion behind
/// the design decision in issue #144: `trigger matches == 0` is a vacuity
/// detector, never a defect detector.
#[test]
fn a_vacuous_cell_and_a_broken_specification_differ_only_across_the_corpus() {
    let sound = cat1_ship_requires_prior_charge();
    let mistyped = cat1_ship_requires_prior_charge_triggered_by("sagashop.order.ship.requeste")
        .expect("a well-formed Event kind instantiates");

    let cancel = adapted("cancel");
    let sound_verdicts = evaluate(&lowered(&sound), &cancel).expect("evaluation succeeds");
    let mistyped_verdicts = evaluate(&lowered(&mistyped), &cancel).expect("evaluation succeeds");

    assert_eq!(
        encoded_verdicts(&sound_verdicts),
        encoded_verdicts(&mistyped_verdicts),
        "the two are supposed to be indistinguishable in the Verdicts; if they are not, this \
         test no longer constructs the case it is about"
    );
    assert_eq!(
        aggregate(censused(&sound, &cancel), &sound_verdicts),
        aggregate(censused(&mistyped, &cancel), &mistyped_verdicts),
        "one capture cannot separate them either, and must not pretend to"
    );

    let corpus = |instance: &PatternInstance| {
        CorpusExercise::of(
            ["happy", "cancel", "hazards"]
                .into_iter()
                .map(|scenario| censused(instance, &adapted(scenario)).trigger),
        )
    };
    assert!(corpus(&sound).was_exercised());
    assert!(!corpus(&mistyped).was_exercised());
}

/// **A legitimately vacuous cell still passes, and is still censused.**
///
/// `cancel.P4` in full. The frozen matrix says `"pass"` and it stays `"pass"`;
/// the census says the trigger kind never occurred on that capture, which is
/// true; and the corpus fold says the Specification is exercised in two of its
/// three captures, which is what makes the first two compatible.
///
/// `Exercised { traces: 3, exercised_in: 2 }` will read as wrong to a reviewer —
/// a *correct* Specification censused as having measured nothing on one capture.
/// That is the point, and it is pinned deliberately: the alternative is a check
/// that fires on every obligation that legitimately never arose, which would
/// break `shared/oracle.ts`'s own reading of the property and the frozen
/// baseline built on it.
#[test]
fn a_legitimately_vacuous_cell_still_passes_and_is_still_censused() {
    let instance = cat1_ship_requires_prior_charge();
    let specification = lowered(&instance);
    let cancel = adapted("cancel");

    let verdicts = evaluate(&specification, &cancel).expect("evaluation succeeds");
    let exercise = censused(&instance, &cancel);

    assert_eq!(frozen_property_expectation("cancel", "P4"), "pass");
    assert!(aggregate(exercise, &verdicts).is_pass());
    assert!(
        matches!(exercise.trigger, TriggerExercise::KindNeverOccurred { .. }),
        "expected the correct Specification to be censused as measuring nothing here, got {:?}",
        exercise.trigger
    );
    assert!(exercise.binding_carriers > 0);

    assert_eq!(
        CorpusExercise::of(
            ["happy", "cancel", "hazards"]
                .into_iter()
                .map(|scenario| censused(&instance, &adapted(scenario)).trigger)
        ),
        CorpusExercise::Exercised {
            traces: NonZeroUsize::new(3).expect("three captures"),
            exercised_in: NonZeroUsize::new(2).expect("happy and hazards"),
        }
    );
}

/// The census matches with the matcher the Verification Core evaluates with.
///
/// `sentinel_compile::ir::SelectorIr` is the one matcher and `core`'s
/// `EventSelector` delegates to it, so this cannot fail by construction today —
/// which is the claim, and it is worth holding over real evidence rather than
/// leaving as a sentence. If someone reintroduces a second matcher, 122
/// SagaShop Events across three captures disagree here before any Verdict does.
#[test]
fn the_census_and_the_core_agree_on_what_a_selector_matches() {
    let instance = cat1_ship_requires_prior_charge();
    let specification = lowered(&instance);

    for scenario in ["happy", "cancel", "hazards"] {
        let evidence = adapted(scenario);
        let core_matches = evidence
            .events()
            .iter()
            .filter(|event| specification.trigger().matches(event))
            .count();
        let core_bindings = evidence
            .events()
            .iter()
            .filter(|event| specification.binding_of(event).is_some())
            .count();

        let exercise = censused(&instance, &evidence);
        let censused_matches = match exercise.trigger {
            TriggerExercise::RaisedObligations { matches, .. }
            | TriggerExercise::MatchedWithoutBinding { matches } => matches.get(),
            TriggerExercise::NoEvidenceExamined
            | TriggerExercise::KindNeverOccurred { .. }
            | TriggerExercise::ValuationNeverMatched { .. } => 0,
        };

        assert_eq!(
            censused_matches, core_matches,
            "{scenario}: selector matching disagrees"
        );
        assert_eq!(
            exercise.binding_carriers, core_bindings,
            "{scenario}: the Binding rule disagrees"
        );
    }
}

/// **A trigger is only reported unexercised once the evidence has settled.**
///
/// The condition `SettledEvidence` asserts, checked against the Core that owns
/// it. An Evaluator that has been handed nothing has not settled; one handed
/// every Event but no source evidence still has not, because a required source
/// that declared nothing leaves the gate closed; one handed both has.
///
/// This matters because the inverse is the same defect one layer up: reporting
/// that a trigger matched nothing while Events are still arriving would be a
/// finding about the clock wearing the costume of a finding about the
/// Specification.
///
/// It asks `evidence_is_settled()` rather than `seal()` deliberately — issue
/// #158 is the separate defect that `seal()` panics on the middle state here,
/// and this test must not depend on its fix.
#[test]
fn a_trigger_is_only_reported_unexercised_once_the_evidence_has_settled() {
    let specification = lowered(&cat1_ship_requires_prior_charge());
    let evidence = adapted("happy");

    let mut evaluator = Evaluator::new(specification);
    assert!(
        !evaluator.evidence_is_settled(),
        "an Evaluator that has seen nothing cannot license a claim about what is absent"
    );

    for event in evidence.events() {
        evaluator
            .step(Input::event(event.clone()))
            .expect("delivery succeeds");
    }
    assert!(
        !evaluator.evidence_is_settled(),
        "Events whose sources have declared no evidence leave Evidence Coverage open"
    );

    for source in evidence.sources().values() {
        evaluator
            .step(Input::source(source.clone()))
            .expect("delivery succeeds");
    }
    assert!(
        evaluator.evidence_is_settled(),
        "a complete capture is exactly the settled evidence the census needs"
    );
}
