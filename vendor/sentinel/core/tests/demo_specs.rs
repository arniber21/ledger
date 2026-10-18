//! Demo Spec pack over vendored SagaShop captures (fixtures slice of #254).
//!
//! Loads `testdata/sagashop/demo-specs/manifest.json` and asserts **alongside**
//! the catalogue instantiations / frozen-matrix helpers in `support` — the same
//! P4 and P8 CAT-1 instances `catalogue_grounding.rs` already proves. This file
//! does **not** re-fold that module's `PropertyOutcome` or the matrix oracle; it
//! only checks the pack's narratives and `coverage_beat` contract against those
//! existing helpers.
//!
//! Non-goals: live #252 promote, dashboard demo mode, forked bootstrap P4,
//! claiming ADR-0019 M0 exit progress (#256).

#![allow(clippy::doc_markdown)] // Product names and file paths read as prose here.

mod support;

use std::path::PathBuf;

use sentinel_compile::catalogue::{PatternId, PatternInstance, SettledEvidence, TriggerExercise};
use sentinel_core::specification::PrecedenceSpecification;
use sentinel_model::VerdictOutcome;
use serde_json::Value;

use support::{adapted, all_cat1_instances, evaluate, frozen_property_expectation, testdata_dir};

/// Allowed `coverage_beat` values in the demo Spec manifest.
const COVERAGE_BEATS: &[&str] = &[
    "obligation_checked",
    "no_obligation_named",
    "matrix_pass_on_hazards",
];

/// Scenarios every demo Spec must speak about.
const SCENARIOS: &[&str] = &["happy", "cancel", "hazards"];

fn manifest_path() -> PathBuf {
    testdata_dir().join("demo-specs").join("manifest.json")
}

fn load_manifest() -> Value {
    let path = manifest_path();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!("demo Spec manifest readable at {}: {error}", path.display())
    });
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("demo Spec manifest is JSON: {error}"))
}

fn required_str<'a>(object: &'a Value, key: &str, context: &str) -> &'a str {
    object
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{context}: missing string field {key:?}"))
}

fn required_object<'a>(object: &'a Value, key: &str, context: &str) -> &'a Value {
    let value = object
        .get(key)
        .unwrap_or_else(|| panic!("{context}: missing object field {key:?}"));
    assert!(
        value.is_object(),
        "{context}: field {key:?} must be an object"
    );
    value
}

fn lowered(instance: &PatternInstance) -> PrecedenceSpecification {
    PrecedenceSpecification::try_from(
        instance
            .ir()
            .as_precedence()
            .expect("CAT-1 demo Specs lower to the precedence monitor"),
    )
    .expect("catalogued CAT-1 IR converts")
}

fn catalogue_instance(property: &str) -> PatternInstance {
    all_cat1_instances()
        .into_iter()
        .find(|(id, _)| *id == property)
        .map_or_else(
            || {
                panic!(
                    "demo Spec catalogue.property {property:?} must resolve to an existing CAT-1 \
                     helper (P4/P6/P8); do not invent a parallel instantiation"
                )
            },
            |(_, instance)| instance,
        )
}

/// Manifest schema + exactly two Specs; beats and matrix cells live in data.
#[test]
fn demo_spec_manifest_names_exactly_two_catalogue_grounded_specs() {
    let manifest = load_manifest();
    assert_eq!(
        required_str(&manifest, "artifactVersion", "manifest"),
        "demo-specs-v1"
    );
    assert_eq!(
        manifest
            .get("part_of")
            .and_then(Value::as_u64)
            .expect("manifest.part_of"),
        254,
        "pack is Part of #254 only — never Closes"
    );
    let note = required_str(&manifest, "adr_0019_note", "manifest");
    assert!(
        note.contains("#256") && note.to_ascii_lowercase().contains("do not count"),
        "manifest must carry ADR-0019/#256 honesty in data, not only in README"
    );

    let specs = manifest
        .get("specs")
        .and_then(Value::as_array)
        .expect("manifest.specs is an array");
    assert_eq!(
        specs.len(),
        2,
        "this slice ships exactly two demo Specs (P4 + P8); no third Spec"
    );

    let mut demo_ids = Vec::new();
    let mut properties = Vec::new();
    for spec in specs {
        let demo_id = required_str(spec, "demo_id", "spec");
        assert!(
            demo_id.starts_with("demo.sagashop."),
            "{demo_id}: stable demo ids stay under demo.sagashop.*"
        );
        demo_ids.push(demo_id.to_owned());

        assert_eq!(
            required_str(spec, "pattern_id", demo_id),
            "precedence.approval"
        );
        let narrative = required_str(spec, "narrative", demo_id);
        assert!(
            !narrative.is_empty() && narrative.len() < 200,
            "{demo_id}: one-line non-expert narrative required"
        );

        let catalogue = required_object(spec, "catalogue", demo_id);
        assert_eq!(required_str(catalogue, "pattern", demo_id), "CAT-1");
        let property = required_str(catalogue, "property", demo_id);
        properties.push(property.to_owned());
        let instantiation_id = required_str(catalogue, "instantiation_id", demo_id);

        let instance = catalogue_instance(property);
        assert_eq!(instance.pattern(), PatternId::PrecedenceApproval);
        assert_eq!(
            lowered(&instance).identity().id(),
            instantiation_id,
            "{demo_id}: catalogue.instantiation_id must match the existing helper"
        );

        let expected = required_object(spec, "expected_matrix", demo_id);
        let beats = required_object(spec, "coverage_beats", demo_id);
        for scenario in SCENARIOS {
            let cell = required_str(expected, scenario, demo_id);
            assert_eq!(
                cell,
                frozen_property_expectation(scenario, property),
                "{demo_id}: expected_matrix.{scenario} must match the frozen matrix (no restated oracle)"
            );
            let beat = required_str(beats, scenario, demo_id);
            assert!(
                COVERAGE_BEATS.contains(&beat),
                "{demo_id}.{scenario}: unknown coverage_beat {beat:?}"
            );
        }
    }

    demo_ids.sort();
    assert_eq!(
        demo_ids,
        [
            "demo.sagashop.fulfill-requires-prior-charge".to_owned(),
            "demo.sagashop.ship-requires-prior-charge".to_owned(),
        ]
    );
    properties.sort();
    assert_eq!(
        properties,
        ["P4".to_owned(), "P8".to_owned()],
        "Spec A is P4 ship←charge; Spec B is distinct P8 cancel-vacuity — not two labels on one row"
    );
}

/// Each coverage_beat is enforced against existing catalogue helpers / evaluation.
#[test]
fn demo_spec_coverage_beats_hold_alongside_catalogue_helpers() {
    let manifest = load_manifest();
    let specs = manifest
        .get("specs")
        .and_then(Value::as_array)
        .expect("manifest.specs");

    for spec in specs {
        let demo_id = required_str(spec, "demo_id", "spec");
        let property = required_str(
            required_object(spec, "catalogue", demo_id),
            "property",
            demo_id,
        );
        let beats = required_object(spec, "coverage_beats", demo_id);
        let instance = catalogue_instance(property);
        let specification = lowered(&instance);

        for scenario in SCENARIOS {
            let beat = required_str(beats, scenario, demo_id);
            let evidence = adapted(scenario);
            let verdicts =
                evaluate(&specification, &evidence).expect("demo Spec evaluation succeeds");
            let exercise = instance.exercise(SettledEvidence::asserted(evidence.events()));

            match beat {
                "obligation_checked" => {
                    assert!(
                        !verdicts.is_empty(),
                        "{demo_id}.{scenario}: obligation_checked requires at least one Binding Verdict"
                    );
                    assert!(
                        verdicts.iter().all(|binding| {
                            binding.verdict().outcome() == VerdictOutcome::Satisfy
                        }),
                        "{demo_id}.{scenario}: obligation_checked requires every Verdict to Satisfy"
                    );
                    assert!(
                        matches!(exercise.trigger, TriggerExercise::RaisedObligations { .. }),
                        "{demo_id}.{scenario}: catalogue census must show raised obligations"
                    );
                }
                "no_obligation_named" => {
                    // Cancel vacuity: named NoObligation / Coverage gap — never
                    // unlabeled Satisfy. Discriminator matches catalogue_grounding's
                    // NoObligation arm without re-folding PropertyOutcome: empty
                    // Verdicts + Binding carriers present + trigger never raised.
                    assert!(
                        verdicts.is_empty(),
                        "{demo_id}.{scenario}: no_obligation_named must emit no Verdicts \
                         (not Satisfy)"
                    );
                    assert!(
                        exercise.binding_carriers > 0,
                        "{demo_id}.{scenario}: Binding carriers must exist so this is \
                         NoObligation, not NoBindings"
                    );
                    assert!(
                        matches!(
                            exercise.trigger,
                            TriggerExercise::KindNeverOccurred { .. }
                                | TriggerExercise::ValuationNeverMatched { .. }
                        ),
                        "{demo_id}.{scenario}: trigger must never have raised an obligation, \
                         got {:?}",
                        exercise.trigger
                    );
                }
                "matrix_pass_on_hazards" => {
                    assert_eq!(
                        *scenario, "hazards",
                        "{demo_id}: matrix_pass_on_hazards is only for the hazards cell"
                    );
                    assert_eq!(
                        frozen_property_expectation("hazards", property),
                        "pass",
                        "{demo_id}: CAT-1 hazards cells are matrix pass — not Violate"
                    );
                    assert!(
                        verdicts.iter().all(|binding| {
                            binding.verdict().outcome() == VerdictOutcome::Satisfy
                        }),
                        "{demo_id}.hazards: matrix pass means Satisfy when Verdicts exist, \
                         never a Violate walkthrough on this pack"
                    );
                    assert!(
                        !verdicts.is_empty(),
                        "{demo_id}.hazards: P4/P8 raise an obligation on hazards (H2 ships / fulfills)"
                    );
                }
                other => panic!("{demo_id}.{scenario}: unknown coverage_beat {other:?}"),
            }
        }
    }
}
