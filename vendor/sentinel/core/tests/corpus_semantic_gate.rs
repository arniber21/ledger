//! #78's semantic gate: the property matrix survives sanitization, end to end.
//!
//! The pipeline under test, per the issue and trace-grab ADR-0013:
//!
//! 1. Convert the vendored `capture-envelope-v1` captures into generic
//!    `trace-corpus-v1` **input** records ([`corpus_support::capture_to_corpus_records`]
//!    — the conversion lives here because trace-grab must never learn
//!    SagaShop's internal format).
//! 2. Adapt that pre-sanitization corpus through the corpus Adapter.
//! 3. Decode the vendored, **sanitized** corpus bundles (trace-grab 0.1.2
//!    output under `testdata/corpus/sagashop/`) through the same Adapter.
//! 4. Assert every property in `scenario-property-matrix.json` holds
//!    identically at step 2 and step 3, and matches the frozen matrix cell.
//!
//! P1/P2/P3/P5/P7 are oracle checks — direct evidence inspection mirroring
//! `core/tests/support`'s, which is where the oracle implementations are
//! canonical. P4/P6/P8 are `PrecedenceSpecification`s evaluated by the real
//! Core, with corpus Event kinds and source identities.
//!
//! This is also the format-contract regression gate: a `trace-corpus-v1` shape
//! change breaks decoding here first, in the repo that cares.

#![allow(clippy::doc_markdown)] // Product names read as prose here.

mod corpus_support;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use sentinel_core::verdict::BindingVerdict;
use sentinel_ingest::corpus::Corpus;
use sentinel_ingest::corpus_adapter::CorpusAdapter;
use sentinel_ingest::evidence::ObservedEvidence;
use sentinel_model::{
    AttributeValue, Event, EventKind, MonitoredApplicationId, SchemaVersion, SourceId,
    SourceIdentity, SourceTier,
};

use sentinel_core::evaluator::Evaluator;
use sentinel_core::specification::{
    AttributeKey, EventSelector, PrecedenceSpecification, RequiredSource, SpecificationId,
    SpecificationIdentity, SpecificationVersion,
};

/// The corpus Adapter configured the way the SagaShop corpus licenses.
///
/// The synchronous-call-graph premise and the tier map are asserted **here**,
/// at the call site, because they are licensed by the SagaShop route contract
/// and the SagaShop deployment's assurance argument — exactly as
/// `core/tests/support::adapt` asserts them for the capture path. A generic
/// corpus gets neither.
fn adapter() -> CorpusAdapter {
    CorpusAdapter::new()
        .with_synchronous_call_graph()
        .with_source_tiers([
            ("sagashop-payment".to_owned(), SourceTier::Anchor),
            ("sagashop-inventory".to_owned(), SourceTier::Anchor),
            ("sagashop-shipping".to_owned(), SourceTier::Anchor),
        ])
}

fn testdata_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("testdata")
}

fn vendored_corpus(scenario: &str) -> Corpus {
    let bytes = std::fs::read(
        testdata_dir()
            .join("corpus")
            .join("sagashop")
            .join(scenario)
            .join("corpus.jsonl"),
    )
    .unwrap_or_else(|error| panic!("vendored {scenario} corpus is readable: {error}"));
    Corpus::from_jsonl(&bytes)
        .unwrap_or_else(|error| panic!("vendored {scenario} corpus decodes: {error}"))
}

fn pre_sanitization_corpus(scenario: &str) -> Corpus {
    let bytes = std::fs::read(
        testdata_dir()
            .join("sagashop")
            .join(format!("ts-{scenario}.json")),
    )
    .unwrap_or_else(|error| panic!("vendored {scenario} capture is readable: {error}"));
    let capture =
        sentinel_ingest::capture::Capture::from_json(&bytes).expect("vendored capture decodes");
    let records = corpus_support::capture_to_corpus_records(&capture)
        .expect("capture converts to corpus records");
    let jsonl = corpus_support::records_to_jsonl(&records);
    Corpus::from_jsonl(jsonl.as_bytes()).expect("converted corpus decodes")
}

/// Adapts a corpus and asserts every Event is Corpus-class, never Tenant.
fn adapt_corpus(corpus: &Corpus) -> ObservedEvidence {
    let evidence = adapter().adapt(corpus).expect("corpus adapts");
    for event in evidence.events() {
        assert_eq!(
            event.data_class(),
            sentinel_model::DataClass::Corpus,
            "the class boundary (ADR-0029) holds for every corpus Event"
        );
    }
    evidence
}

/// Reads the frozen matrix cell, ported from `core/tests/support` (the
/// matrix-parsing helper is duplicated rather than shared because the two
/// crates' test trees are separate; the artifact itself is the shared truth).
fn frozen_property_expectation(scenario: &str, property: &str) -> String {
    let text = std::fs::read_to_string(
        testdata_dir()
            .join("sagashop")
            .join("scenario-property-matrix.json"),
    )
    .expect("frozen matrix is readable");
    let scenarios = text
        .split_once("\"scenarios\"")
        .expect("frozen matrix declares scenarios")
        .1;
    let row = scenarios
        .split_once(&format!("\"{scenario}\""))
        .unwrap_or_else(|| panic!("frozen matrix declares scenario {scenario}"))
        .1;
    let row = &row[..row.find('}').expect("scenario row is closed")];
    let cell = row
        .split_once(&format!("\"{property}\""))
        .unwrap_or_else(|| panic!("frozen matrix declares {scenario}.{property}"))
        .1;
    let cell = cell
        .split_once(':')
        .expect("property cell has a value")
        .1
        .trim_start();
    let cell = cell
        .strip_prefix('"')
        .expect("property cell holds a quoted value");
    cell[..cell.find('"').expect("property cell is closed")].to_owned()
}

// ---------------------------------------------------------------------------
// Oracle checks (P1/P2/P3/P5/P7) — mirroring core/tests/support's
// ---------------------------------------------------------------------------

fn attr_is(event: &Event, key: &str, value: &str) -> bool {
    event
        .producer()
        .attributes()
        .get(&AttributeKey::new(key).expect("fixture"))
        == Some(&AttributeValue::String(value.to_owned()))
}

fn order_of(event: &Event) -> Option<String> {
    match event
        .producer()
        .attributes()
        .get(&AttributeKey::new("sagashop.order.id").expect("fixture"))
    {
        Some(AttributeValue::String(order)) => Some(order.clone()),
        _ => None,
    }
}

/// P1: no ship may depart from a cancelled order.
fn no_ship_from_cancelled(evidence: &ObservedEvidence) -> bool {
    !evidence.events().iter().any(|event| {
        event.producer().kind().as_str() == "order.ship.requested"
            && attr_is(event, "sagashop.order.prev_state", "CANCELLED")
    })
}

/// P2: at most one successful charge per order.
fn at_most_one_successful_charge(evidence: &ObservedEvidence) -> bool {
    let mut charges_per_order: BTreeMap<String, usize> = BTreeMap::new();
    for event in evidence.events() {
        if event.producer().kind().as_str() == "payment.charge.completed"
            && attr_is(event, "sagashop.payment.status", "success")
        {
            if let Some(order) = order_of(event) {
                *charges_per_order.entry(order).or_default() += 1;
            }
        }
    }
    !charges_per_order.values().any(|count| *count > 1)
}

/// P3: cancel is terminal (a cancelled order must go to REFUNDED).
fn cancel_is_terminal(evidence: &ObservedEvidence) -> bool {
    !evidence.events().iter().any(|event| {
        attr_is(event, "sagashop.order.prev_state", "CANCELLED")
            && !attr_is(event, "sagashop.order.state", "REFUNDED")
    })
}

/// P5: inventory `stock_after` must strictly decrease per item along causal
/// order; all-concurrent reserves with equal stock fail (H3).
fn inventory_monotonic_decrement(evidence: &ObservedEvidence) -> bool {
    let by_id: BTreeMap<String, &Event> = evidence
        .events()
        .iter()
        .map(|event| (event.id().to_string(), event))
        .collect();

    let mut reserves_by_item: BTreeMap<String, Vec<&Event>> = BTreeMap::new();
    for event in evidence.events() {
        if event.producer().kind().as_str() == "inventory.reserve.completed"
            && event
                .producer()
                .attributes()
                .get(&AttributeKey::new("sagashop.inventory.reserved").expect("fixture"))
                == Some(&AttributeValue::Bool(true))
        {
            if let Some(AttributeValue::String(item)) = event
                .producer()
                .attributes()
                .get(&AttributeKey::new("sagashop.inventory.item").expect("fixture"))
            {
                reserves_by_item
                    .entry(item.clone())
                    .or_default()
                    .push(event);
            }
        }
    }

    let stock_after = |event: &Event| -> i64 {
        match event
            .producer()
            .attributes()
            .get(&AttributeKey::new("sagashop.inventory.stock_after").expect("fixture"))
        {
            Some(AttributeValue::Integer(value)) => *value,
            _ => i64::MAX,
        }
    };
    let causal_reaches = |earlier: &str, later: &str| -> bool {
        let mut visited = BTreeSet::new();
        let mut stack = vec![later.to_owned()];
        while let Some(current) = stack.pop() {
            if current == earlier {
                return true;
            }
            if !visited.insert(current.clone()) {
                continue;
            }
            if let Some(event) = by_id.get(&current) {
                for predecessor in event.causal_predecessors() {
                    stack.push(predecessor.to_string());
                }
            }
        }
        false
    };

    for reserves in reserves_by_item.values() {
        if reserves.len() <= 1 {
            continue;
        }
        let mut any_causal_pair = false;
        for left in reserves {
            for right in reserves {
                if std::ptr::eq(left, right) {
                    continue;
                }
                if causal_reaches(left.id().as_str(), right.id().as_str()) {
                    any_causal_pair = true;
                    if stock_after(left) <= stock_after(right) {
                        return false;
                    }
                }
            }
        }
        if !any_causal_pair {
            let stocks: Vec<i64> = reserves.iter().map(|event| stock_after(event)).collect();
            for index in 0..stocks.len() {
                for other in index + 1..stocks.len() {
                    if stocks[index] == stocks[other] {
                        return false;
                    }
                }
            }
        }
    }
    true
}

/// P7: every reserve has a prior create for the same order.
fn reserve_requires_prior_create(evidence: &ObservedEvidence) -> bool {
    let by_id: BTreeMap<String, &Event> = evidence
        .events()
        .iter()
        .map(|event| (event.id().to_string(), event))
        .collect();
    let causal_reaches = |earlier: &str, later: &str| -> bool {
        let mut visited = BTreeSet::new();
        let mut stack = vec![later.to_owned()];
        while let Some(current) = stack.pop() {
            if current == earlier {
                return true;
            }
            if !visited.insert(current.clone()) {
                continue;
            }
            if let Some(event) = by_id.get(&current) {
                for predecessor in event.causal_predecessors() {
                    stack.push(predecessor.to_string());
                }
            }
        }
        false
    };

    for event in evidence.events() {
        if event.producer().kind().as_str() != "inventory.reserve.requested" {
            continue;
        }
        let Some(order) = order_of(event) else {
            continue;
        };
        let has_prior_create = evidence.events().iter().any(|create| {
            create.producer().kind().as_str() == "order.create.requested"
                && order_of(create).is_some_and(|candidate| candidate == order)
                && causal_reaches(create.id().as_str(), event.id().as_str())
        });
        if !has_prior_create {
            return false;
        }
    }
    true
}

fn oracle_property(property: &str, evidence: &ObservedEvidence) -> bool {
    match property {
        "P1" => no_ship_from_cancelled(evidence),
        "P2" => at_most_one_successful_charge(evidence),
        "P3" => cancel_is_terminal(evidence),
        "P5" => inventory_monotonic_decrement(evidence),
        "P7" => reserve_requires_prior_create(evidence),
        other => panic!("unknown oracle property {other}"),
    }
}

// ---------------------------------------------------------------------------
// Precedence properties (P4/P6/P8) — the real Core over corpus Events
// ---------------------------------------------------------------------------

fn source(service: &str) -> SourceIdentity {
    SourceIdentity::new(
        MonitoredApplicationId::new(service).expect("service name is not empty"),
        SourceId::new(sentinel_ingest::corpus_adapter::SOURCE_ID).expect("constant source id"),
    )
}

fn required(services: &[(&str, SourceTier)]) -> BTreeSet<RequiredSource> {
    services
        .iter()
        .map(|(service, tier)| RequiredSource::new(source(service), *tier))
        .collect()
}

/// P4 over corpus kinds: `order.ship.requested` requires a prior successful
/// `payment.charge.completed`, bound by order id.
fn ship_requires_prior_charge() -> PrecedenceSpecification {
    let mut successful_charge = BTreeMap::new();
    successful_charge.insert(
        AttributeKey::new("sagashop.payment.status").expect("fixture"),
        AttributeValue::String("success".to_owned()),
    );
    PrecedenceSpecification::new(
        SpecificationIdentity::new(
            SpecificationId::new("corpus.p4.ship-requires-prior-charge").expect("fixture"),
            SpecificationVersion::new("v1").expect("fixture"),
        ),
        AttributeKey::new("sagashop.order.id").expect("fixture"),
        EventSelector::new(
            EventKind::new("order.ship.requested").expect("constant kind"),
            BTreeMap::new(),
        ),
        EventSelector::new(
            EventKind::new("payment.charge.completed").expect("constant kind"),
            successful_charge,
        ),
        required(&[
            ("sagashop-order", SourceTier::Contributing),
            ("sagashop-payment", SourceTier::Anchor),
        ]),
        SchemaVersion::new("trace-corpus-v1").expect("constant schema version"),
    )
}

/// P6: `order.charge.requested` requires a prior successful reserve.
fn charge_requires_prior_reserve() -> PrecedenceSpecification {
    let mut successful_reserve = BTreeMap::new();
    successful_reserve.insert(
        AttributeKey::new("sagashop.inventory.reserved").expect("fixture"),
        AttributeValue::Bool(true),
    );
    PrecedenceSpecification::new(
        SpecificationIdentity::new(
            SpecificationId::new("corpus.p6.charge-requires-prior-reserve").expect("fixture"),
            SpecificationVersion::new("v1").expect("fixture"),
        ),
        AttributeKey::new("sagashop.order.id").expect("fixture"),
        EventSelector::new(
            EventKind::new("order.charge.requested").expect("constant kind"),
            BTreeMap::new(),
        ),
        EventSelector::new(
            EventKind::new("inventory.reserve.completed").expect("constant kind"),
            successful_reserve,
        ),
        required(&[
            ("sagashop-order", SourceTier::Contributing),
            ("sagashop-inventory", SourceTier::Anchor),
        ]),
        SchemaVersion::new("trace-corpus-v1").expect("constant schema version"),
    )
}

/// P8: `order.fulfill.requested` requires a prior successful charge.
fn fulfill_requires_prior_charge() -> PrecedenceSpecification {
    let mut successful_charge = BTreeMap::new();
    successful_charge.insert(
        AttributeKey::new("sagashop.payment.status").expect("fixture"),
        AttributeValue::String("success".to_owned()),
    );
    PrecedenceSpecification::new(
        SpecificationIdentity::new(
            SpecificationId::new("corpus.p8.fulfill-requires-prior-charge").expect("fixture"),
            SpecificationVersion::new("v1").expect("fixture"),
        ),
        AttributeKey::new("sagashop.order.id").expect("fixture"),
        EventSelector::new(
            EventKind::new("order.fulfill.requested").expect("constant kind"),
            BTreeMap::new(),
        ),
        EventSelector::new(
            EventKind::new("payment.charge.completed").expect("constant kind"),
            successful_charge,
        ),
        required(&[
            ("sagashop-order", SourceTier::Contributing),
            ("sagashop-payment", SourceTier::Anchor),
        ]),
        SchemaVersion::new("trace-corpus-v1").expect("constant schema version"),
    )
}

/// The number of Bindings a precedence Specification creates over evidence.
///
/// The gate asserts binding counts pre==post separately from outcomes,
/// because `precedence_passes` is vacuously true over zero Bindings — a
/// sanitizer that destroyed the binding attribute would leave every verdict
/// absent and every property "passing". Count equality is what makes binding
/// creation itself part of the equivalence under test.
fn precedence_binding_count(
    specification: &PrecedenceSpecification,
    evidence: &ObservedEvidence,
) -> usize {
    sentinel_core::evaluate(
        Evaluator::new(specification.clone()),
        evidence.events(),
        evidence.sources(),
    )
    .expect("evaluation succeeds")
    .len()
}

/// Pass = every Binding's verdict is Satisfy, mirroring the e2e oracle's
/// aggregation rule. (The matrix's `fail` cells are the hazards scenarios,
/// where P4/P6/P8 still pass because the hazards violate P1/P2/P3/P5/P7.)
fn precedence_passes(specification: &PrecedenceSpecification, evidence: &ObservedEvidence) -> bool {
    sentinel_core::evaluate(
        Evaluator::new(specification.clone()),
        evidence.events(),
        evidence.sources(),
    )
    .expect("evaluation succeeds")
    .iter()
    .all(|verdict: &BindingVerdict| {
        verdict.verdict().outcome() == sentinel_model::VerdictOutcome::Satisfy
    })
}

fn precedence_property(property: &str, evidence: &ObservedEvidence) -> bool {
    match property {
        "P4" => precedence_passes(&ship_requires_prior_charge(), evidence),
        "P6" => precedence_passes(&charge_requires_prior_reserve(), evidence),
        "P8" => precedence_passes(&fulfill_requires_prior_charge(), evidence),
        other => panic!("unknown precedence property {other}"),
    }
}

fn property_holds(property: &str, evidence: &ObservedEvidence) -> bool {
    match property {
        "P4" | "P6" | "P8" => precedence_property(property, evidence),
        "P1" | "P2" | "P3" | "P5" | "P7" => oracle_property(property, evidence),
        other => panic!("unknown property {other}"),
    }
}

// ---------------------------------------------------------------------------
// The gate
// ---------------------------------------------------------------------------

/// The gate proper: pre-sanitization outcome == post-sanitization outcome ==
/// frozen matrix cell, for all eight properties and all three scenarios.
#[test]
fn every_property_survives_sanitization_and_matches_the_frozen_matrix() {
    for scenario in ["happy", "cancel", "hazards"] {
        let pre = adapt_corpus(&pre_sanitization_corpus(scenario));
        let post = adapt_corpus(&vendored_corpus(scenario));

        // Sanitization must not change the record count: topology survives.
        assert_eq!(
            pre.events().len(),
            post.events().len(),
            "{scenario}: sanitization preserves record count"
        );

        for property in ["P1", "P2", "P3", "P4", "P5", "P6", "P7", "P8"] {
            let expected_pass = frozen_property_expectation(scenario, property) == "pass";
            let pre_pass = property_holds(property, &pre);
            let post_pass = property_holds(property, &post);
            assert_eq!(
                pre_pass, expected_pass,
                "{scenario}.{property}: pre-sanitization outcome matches the frozen matrix"
            );
            assert_eq!(
                post_pass, expected_pass,
                "{scenario}.{property}: post-sanitization outcome matches the frozen matrix"
            );

            // Precedence outcomes are vacuously true over zero Bindings, so
            // binding creation is asserted directly: sanitization must not
            // destroy it.
            if matches!(property, "P4" | "P6" | "P8") {
                let specification = match property {
                    "P4" => ship_requires_prior_charge(),
                    "P6" => charge_requires_prior_reserve(),
                    _ => fulfill_requires_prior_charge(),
                };
                assert_eq!(
                    precedence_binding_count(&specification, &pre),
                    precedence_binding_count(&specification, &post),
                    "{scenario}.{property}: sanitization preserves binding count"
                );
            }
        }
    }

    // Fixture truth: the happy corpus ships exactly one order, so P4 creates
    // exactly one Binding; the hazards corpus ships only the H2 order (the
    // double-charged H1 order never ships), so P4 creates one Binding there
    // too. Zero bindings anywhere would mean the Core path is unexercised.
    for (scenario, expected_bindings) in [("happy", 1), ("hazards", 1)] {
        let post = adapt_corpus(&vendored_corpus(scenario));
        assert_eq!(
            precedence_binding_count(&ship_requires_prior_charge(), &post),
            expected_bindings,
            "{scenario}: P4 binds the expected number of orders"
        );
    }
}

/// The corpus Event kinds are the analysis vocabulary — the `name` each
/// record carries, plus the phase — and survive sanitization because `name`
/// passes verbatim. This pins the format contract's most load-bearing
/// pass-verbatim field from the consuming side.
#[test]
fn event_kinds_survive_sanitization_verbatim() {
    for scenario in ["happy", "cancel", "hazards"] {
        let pre = adapt_corpus(&pre_sanitization_corpus(scenario));
        let post = adapt_corpus(&vendored_corpus(scenario));
        let pre_kinds: BTreeSet<String> = pre
            .events()
            .iter()
            .map(|event| event.producer().kind().as_str().to_owned())
            .collect();
        let post_kinds: BTreeSet<String> = post
            .events()
            .iter()
            .map(|event| event.producer().kind().as_str().to_owned())
            .collect();
        assert_eq!(pre_kinds, post_kinds, "{scenario}: kinds survive");
        assert!(
            pre_kinds.contains("payment.charge.completed"),
            "{scenario}: the analysis vocabulary is present"
        );
    }
}

/// Token equality preserves the binding: every order id that appears in both
/// a reserve and a create matches itself across those Events. This is the
/// specific relational fact over-redaction would destroy.
#[test]
fn token_equality_preserves_binding_across_events() {
    let evidence = adapt_corpus(&vendored_corpus("happy"));
    let orders_with_reserve: BTreeSet<Option<String>> = evidence
        .events()
        .iter()
        .filter(|event| event.producer().kind().as_str() == "inventory.reserve.requested")
        .map(order_of)
        .collect();
    let orders_with_create: BTreeSet<Option<String>> = evidence
        .events()
        .iter()
        .filter(|event| event.producer().kind().as_str() == "order.create.requested")
        .map(order_of)
        .collect();
    for order in orders_with_reserve {
        assert!(
            orders_with_create.contains(&order),
            "every reserved order's token equals its created order's token"
        );
    }
}

/// The gate has teeth: an over-broad `drop` of exactly the categorical
/// attributes makes the gate **fail**, not pass vacuously.
///
/// The `over-dropped` bundle is the hazards input under a policy that drops
/// `sagashop.order.prev_state` (and the other categorical attributes). The
/// hazards scenario's P1 cell is `fail` precisely because its H2 witness —
/// a ship whose `prev_state` is `CANCELLED` — must be visible. With the
/// attribute dropped the witness is destroyed and P1 flips to a silent pass,
/// so the main gate's core assertion — post-sanitization outcome equals the
/// frozen matrix cell — fails over this bundle. That is the gate having
/// teeth: an over-broad policy cannot hide behind vacuous passes.
#[test]
fn an_over_broad_drop_makes_the_gate_fail() {
    let clean = adapt_corpus(&vendored_corpus("hazards"));
    let dropped = adapt_corpus(&vendored_corpus("over-dropped"));

    // The witness exists in the clean bundle: the hazards scenario's P1 cell
    // is `fail` because of it.
    assert!(
        !property_holds("P1", &clean),
        "clean hazards P1 fails, matching the frozen matrix's fail cell"
    );
    let witness_visible = clean.events().iter().any(|event| {
        event.producer().kind().as_str() == "order.ship.requested"
            && attr_is(event, "sagashop.order.prev_state", "CANCELLED")
    });
    assert!(
        witness_visible,
        "the clean hazards bundle carries the CANCELLED->SHIPPED witness"
    );

    // With prev_state dropped, the witness is gone and P1 flips to a vacuous
    // pass — the exact silent-wrong failure the gate exists to catch.
    assert!(
        property_holds("P1", &dropped),
        "dropping prev_state flips P1 to a vacuous pass"
    );

    // And therefore the gate itself fails over this bundle: the
    // post-sanitization outcome no longer matches the frozen matrix cell.
    let expected_pass = frozen_property_expectation("hazards", "P1") == "pass";
    assert_ne!(
        expected_pass,
        property_holds("P1", &dropped),
        "the gate's pre==post==matrix assertion breaks over the over-dropped bundle"
    );

    // The same break shows up as an attribute-set difference, for the same
    // reason: the evidence the property binds on is gone.
    let dropped_witness_visible = dropped.events().iter().any(|event| {
        event.producer().kind().as_str() == "order.ship.requested"
            && event
                .producer()
                .attributes()
                .contains_key(&AttributeKey::new("sagashop.order.prev_state").expect("fixture"))
    });
    assert!(
        !dropped_witness_visible,
        "the witness is invisible in the over-dropped bundle"
    );
}
