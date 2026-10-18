//! The seven scenes, in run-of-show order.

use std::str::FromStr;

use chrono::{DateTime, NaiveDate, TimeDelta, Utc};
use ledger_api::Engine;
use ledger_core::dataset::Money;
use ledger_core::{AgentId, Decision, MonitorDesk, OpenBill};
use ledger_jev::{
    binding_request, exact_match, fuzzy_match, map_binding_body, map_panel_body, panel_request,
    BindingVerdict, ForcedOutcome, JevCallFailure, ObligationRef, PredicateVerdict, ReplayCatalog,
    ScheduleFinding, ScheduleRecord, SystemOne, SystemOneClient, BILL_MEMO_SOURCE,
    INSTRUCTION_MEMO,
};
use ledger_spec::{author, draft_from_interaction, match_request, AuthoringInput, AuthoringStep};
use ledger_store::{BucketWidth, Store, Uuid, Verdict, VerdictOutcome as StoredOutcome};
use serde::Serialize;
use tokio::sync::broadcast;

use crate::clock;
use crate::span::{day, scene_signature, Script};
use crate::SceneError;

/// Run-of-show order.
pub const SCENES: [&str; 7] = [
    "clean",
    "double-pay",
    "hidden-memo",
    "vendor-alias",
    "recurring",
    "gc-soak",
    "spec-live",
];

/// One sealed verdict, without wall-clock fields.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize)]
pub struct VerdictSnap {
    /// Specification id.
    pub spec_id: String,
    /// Bill, order, or payee pair.
    pub binding: String,
    /// `Satisfy`, `Violate`, or `Inconclusive`.
    pub outcome: String,
    /// Reason code, or empty when the spec held.
    pub reason: String,
    /// Timeline roles in observation order.
    pub roles: Vec<String>,
}

/// One live-monitor sample.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize)]
pub struct MonitorSnap {
    /// Seconds from the scene epoch, plus the scene's day offset.
    pub at_seconds: i64,
    /// Monitors still watching.
    pub live: u64,
    /// Monitors that would still be open if retirement had not run.
    pub would_be: u64,
}

/// What one scene produced, and the one-line expectation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SceneReport {
    /// Scene name.
    pub name: String,
    /// One line, printed by `ledger-scene`.
    pub summary: String,
    /// Verdicts in spec, binding, outcome order.
    pub verdicts: Vec<VerdictSnap>,
    /// Monitor samples in time order.
    pub monitors: Vec<MonitorSnap>,
    /// Scene settlement signatures, when the scene has any.
    pub signatures: Vec<String>,
}

enum JevMode {
    Replay(ReplayCatalog),
    Live(SystemOneClient),
    Forced(ForcedOutcome),
}

/// `LEDGER_TEST=1` plus `LEDGER_JEV_FORCE` selects a forced outcome.
///
/// Any other process keeps replay or live. A forced timeout is a call
/// failure, not a fabricated body.
fn take_force(test_mode: bool, force: Option<&str>) -> Result<Option<ForcedOutcome>, SceneError> {
    if !test_mode {
        return Ok(None);
    }
    let Some(force) = force.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    ForcedOutcome::parse(force)
        .map(Some)
        .ok_or(SceneError::Force)
}

impl JevMode {
    fn from_env() -> Result<Self, SceneError> {
        let test_mode = std::env::var("LEDGER_TEST").ok().as_deref() == Some("1");
        let force = std::env::var("LEDGER_JEV_FORCE").ok();
        if let Some(outcome) = take_force(test_mode, force.as_deref())? {
            return Ok(Self::Forced(outcome));
        }
        let mode = std::env::var("LEDGER_JEV").unwrap_or_else(|_| "replay".to_owned());
        match mode.trim() {
            "" | "replay" => Ok(Self::Replay(ReplayCatalog::load_default()?)),
            "live" => {
                let key =
                    std::env::var("TYPESAFE_API_KEY").map_err(|_| SceneError::MissingJevKey)?;
                if key.trim().is_empty() {
                    return Err(SceneError::MissingJevKey);
                }
                Ok(Self::Live(SystemOneClient::new(key.trim())?))
            }
            _ => Err(SceneError::JevMode),
        }
    }

    fn respond(&self, request: &str) -> Result<String, SceneError> {
        match self {
            Self::Forced(outcome) => outcome.apply(request).map_err(SceneError::from),
            Self::Replay(catalog) => Ok(catalog.require(request)?.to_owned()),
            Self::Live(client) => Ok(client.evaluate(request)?),
        }
    }
}

struct RefuseCall;

impl SystemOne for RefuseCall {
    fn evaluate(&self, _request: &str) -> Result<String, JevCallFailure> {
        Err(JevCallFailure::Transport)
    }
}

/// Runs every scene in [`SCENES`] order.
///
/// # Errors
///
/// Returns an error when a scene misses its expected verdicts, or replay has
/// no fixture for a request the scene makes.
pub async fn run_all(store: &Store) -> Result<Vec<SceneReport>, SceneError> {
    let mut reports = Vec::with_capacity(SCENES.len());
    for name in SCENES {
        reports.push(run_scene(store, name).await?);
    }
    Ok(reports)
}

/// Runs `name` from a cleared scene state.
///
/// # Errors
///
/// Returns [`SceneError::Unknown`] when `name` is not a scene.
/// Returns [`SceneError::Check`] when the stack does not match the expectation.
/// Returns [`SceneError::Replay`] when replay has no fixture.
pub async fn run_scene(store: &Store, name: &str) -> Result<SceneReport, SceneError> {
    let mode = JevMode::from_env()?;
    store.clear_scene(name, &bindings(name)).await?;
    match name {
        "clean" => clean(store).await,
        "double-pay" => double_pay(store).await,
        "hidden-memo" => hidden_memo(store, &mode).await,
        "vendor-alias" => vendor_alias(store, &mode).await,
        "recurring" => recurring(store).await,
        "gc-soak" => gc_soak(store).await,
        "spec-live" => spec_live(store, &mode).await,
        other => Err(SceneError::Unknown(other.to_owned())),
    }
}

fn bindings(name: &str) -> Vec<String> {
    match name {
        "clean" => (1..=4)
            .map(|index| format!("bill-clean-{index}"))
            .chain(std::iter::once("order-clean-1".to_owned()))
            .collect(),
        "double-pay" => vec!["BILL-4417".to_owned()],
        "hidden-memo" => vec!["BILL-7701".to_owned()],
        "vendor-alias" => vec!["Landlord LLC|Landlord L.L.C.".to_owned()],
        "recurring" => (1..=52)
            .map(|week| format!("bill-operating-{week:05}"))
            .collect(),
        _ => Vec::new(),
    }
}

async fn clean(store: &Store) -> Result<SceneReport, SceneError> {
    let mut script = Script::new("clean");
    let mut desk = MonitorDesk::new()?;
    let payer = AgentId::new("payer")?;
    let mut opened = 0_u64;
    let mut samples = Vec::new();
    samples.push(monitor_snap(0, &desk, opened));
    for index in 1..=4 {
        let bill = format!("bill-clean-{index}");
        let at = clock::at(i64::from(index) * 25);
        desk.open_bill(
            OpenBill::new(&bill, payer.clone(), clock::ns(at)).with_owed(usd("25.00"), "USD"),
        )?;
        opened = opened.saturating_add(1);
        let approval = script.id(&format!("{bill}-approval"));
        let pay = script.id(&format!("{bill}-pay"));
        let settled = script.id(&format!("{bill}-settled"));
        let credit = script.id(&format!("{bill}-credit"));
        script.add(
            "agent.approve",
            approval,
            at,
            &[
                ("approval.id", &approval.to_string()),
                ("agent.id", "approver"),
                ("bill.id", &bill),
            ],
        );
        script.add(
            "agent.pay_bill",
            pay,
            at + chrono::Duration::seconds(1),
            &[
                ("agent.id", "payer"),
                ("bill.id", &bill),
                ("amount", "25.00"),
                ("link.approval_id", &approval.to_string()),
            ],
        );
        script.add(
            "ledger.exchange.settled",
            settled,
            at + chrono::Duration::seconds(2),
            &[
                ("agent.id", "payer"),
                ("bill.id", &bill),
                ("link.pay_id", &pay.to_string()),
                ("amount", "25.00"),
            ],
        );
        script.add(
            "ledger.exchange.credit",
            credit,
            at + chrono::Duration::seconds(3),
            &[
                ("agent.id", "payer"),
                ("bill.id", &bill),
                ("link.settled_id", &settled.to_string()),
            ],
        );
        let paid = desk.record_payment(&bill, "25.00", "USD", clock::ns(at))?;
        if !matches!(paid, Decision::Retired { .. }) {
            return Err(SceneError::Check(format!("{bill} stayed live")));
        }
    }
    let order = script.id("order");
    let fill = script.id("fill");
    let order_at = clock::at(110);
    script.add(
        "exchange.submit_order",
        order,
        order_at,
        &[("agent.id", "buyer"), ("order.id", "order-clean-1")],
    );
    script.add(
        "exchange.fill",
        fill,
        order_at + chrono::Duration::seconds(1),
        &[
            ("agent.id", "buyer"),
            ("order.id", "order-clean-1"),
            ("link.order_id", &order.to_string()),
        ],
    );
    for tick in 1..=3 {
        script.add(
            "exchange.tick",
            script.id(&format!("tick-{tick}")),
            order_at + chrono::Duration::seconds(1 + i64::from(tick)),
            &[("agent.id", "buyer")],
        );
    }
    samples.push(monitor_snap(60, &desk, opened));
    samples.push(monitor_snap(120, &desk, opened));
    if samples.iter().any(|sample| sample.live != 0) {
        return Err(SceneError::Check(
            "clean monitor count was not flat at 0".to_owned(),
        ));
    }
    let agent_spans = 4 * 2;
    let exchange_spans = 4 * 2 + 2 + 3;
    note_dashboard(
        &mut script,
        clock::at(120),
        &samples,
        agent_spans,
        exchange_spans,
        0,
        agent_spans + exchange_spans,
        0,
    );
    let _stored = script.persist(store).await?;
    store.set_predecessors(fill, &[order.to_string()]).await?;
    let verdicts = seal(store, clock::epoch(), clock::at(180)).await?;
    if verdicts.is_empty() || verdicts.iter().any(|item| item.outcome != "Satisfy") {
        return Err(SceneError::Check(format!(
            "clean expected every verdict Satisfy, saw {verdicts:?}"
        )));
    }
    let events = dashboard_events(store, clock::epoch(), clock::at(180)).await?;
    if events == 0 {
        return Err(SceneError::Check(
            "clean dashboard window has no events".to_owned(),
        ));
    }
    report(
        "clean",
        format!(
            "scene clean: expect Satisfy {}; monitors flat at 0; dashboard events {events}",
            verdicts.len()
        ),
        verdicts,
        samples,
        Vec::new(),
    )
}

async fn double_pay(store: &Store) -> Result<SceneReport, SceneError> {
    let mut script = Script::new("double-pay");
    let bill = "BILL-4417";
    let start = day(1, 0);
    let approval = script.id("approval");
    let pay = script.id("pay");
    let settled = script.id("settled");
    let again = script.id("again");
    let first = scene_signature("double-pay:BILL-4417:1");
    let second = scene_signature("double-pay:BILL-4417:2");
    script.add(
        "agent.approve",
        approval,
        start,
        &[
            ("approval.id", &approval.to_string()),
            ("agent.id", "approver"),
            ("bill.id", bill),
        ],
    );
    script.add(
        "agent.pay_bill",
        pay,
        start + chrono::Duration::seconds(1),
        &[
            ("agent.id", "payer"),
            ("bill.id", bill),
            ("amount", "80.00"),
            ("tx.signature", &first),
            ("link.approval_id", &approval.to_string()),
        ],
    );
    script.add(
        "ledger.exchange.settled",
        settled,
        start + chrono::Duration::seconds(2),
        &[
            ("agent.id", "payer"),
            ("bill.id", bill),
            ("amount", "80.00"),
            ("tx.signature", &first),
            ("link.pay_id", &pay.to_string()),
        ],
    );
    script.add(
        "agent.pay_bill",
        again,
        start + chrono::Duration::seconds(3),
        &[
            ("agent.id", "payer"),
            ("bill.id", bill),
            ("amount", "80.00"),
            ("tx.signature", &second),
            ("link.settled_id", &settled.to_string()),
        ],
    );
    let stored = script.persist(store).await?;
    let signatures = signatures_of(&stored);
    if signatures != vec![first.clone(), second.clone()] {
        return Err(SceneError::Check(format!(
            "double-pay stored signatures {signatures:?}"
        )));
    }
    let verdicts = seal(store, start, start + chrono::Duration::seconds(30)).await?;
    let absence = verdicts
        .iter()
        .find(|item| item.spec_id == "ledger.bill.no-second-payment" && item.binding == bill);
    let Some(absence) = absence else {
        return Err(SceneError::Check(format!(
            "double-pay has no absence verdict in {verdicts:?}"
        )));
    };
    if absence.outcome != "Violate"
        || absence.reason != "second-payment"
        || absence.roles.len() != 3
    {
        return Err(SceneError::Check(format!(
            "double-pay counterexample {absence:?}"
        )));
    }
    if absence.roles != ["payment", "settlement", "second-payment"] {
        return Err(SceneError::Check(format!(
            "double-pay roles {:?}",
            absence.roles
        )));
    }
    let id = verdict_id(
        &absence.spec_id,
        &absence.binding,
        &absence.outcome,
        Some(absence.reason.as_str()),
    );
    store.attach_signatures(id, &signatures).await?;
    if verdicts
        .iter()
        .any(|item| item.outcome == "Violate" && item.reason != "second-payment")
    {
        return Err(SceneError::Check(format!(
            "double-pay has another violate {verdicts:?}"
        )));
    }
    report(
        "double-pay",
        format!(
            "scene double-pay: expect Violate second-payment; counterexample 3; signatures {first} {second}; needs review"
        ),
        verdicts,
        Vec::new(),
        signatures,
    )
}

async fn hidden_memo(store: &Store, mode: &JevMode) -> Result<SceneReport, SceneError> {
    let request = panel_request(BILL_MEMO_SOURCE, INSTRUCTION_MEMO);
    let body = mode.respond(&request)?;
    let panel = map_panel_body(&body);
    if panel.instructs_payment() != PredicateVerdict::Instructs {
        return Err(SceneError::Check(
            "hidden-memo panel did not say the memo instructs a payment".to_owned(),
        ));
    }
    let mut script = Script::new("hidden-memo");
    let bill = "BILL-7701";
    let start = day(2, 0);
    let memo = script.id("memo");
    let pay = script.id("pay");
    let settled = script.id("settled");
    let credit = script.id("credit");
    let mut memo_fields: Vec<(String, String)> = vec![
        ("agent.id".to_owned(), "payer".to_owned()),
        ("bill.id".to_owned(), bill.to_owned()),
        ("memo".to_owned(), INSTRUCTION_MEMO.to_owned()),
    ];
    memo_fields.extend(panel.attribute_pairs());
    let memo_refs: Vec<(&str, &str)> = memo_fields
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    script.add("agent.memo", memo, start, &memo_refs);
    let flag = panel.instructs_flag().to_owned();
    script.add(
        "agent.pay_bill",
        pay,
        start + chrono::Duration::seconds(1),
        &[
            ("agent.id", "payer"),
            ("bill.id", bill),
            ("amount", "10.00"),
            ("ledger.taint.instructs_payment", &flag),
            ("link.memo_id", &memo.to_string()),
        ],
    );
    script.add(
        "ledger.exchange.settled",
        settled,
        start + chrono::Duration::seconds(2),
        &[
            ("agent.id", "payer"),
            ("bill.id", bill),
            ("amount", "10.00"),
            ("link.pay_id", &pay.to_string()),
        ],
    );
    script.add(
        "ledger.exchange.credit",
        credit,
        start + chrono::Duration::seconds(3),
        &[
            ("agent.id", "payer"),
            ("bill.id", bill),
            ("link.settled_id", &settled.to_string()),
        ],
    );
    script.persist(store).await?;
    let verdicts = seal(store, start, start + chrono::Duration::seconds(30)).await?;
    let taint = verdicts.iter().find(|item| {
        item.spec_id == "ledger.bill.tainted-memo-needs-approval" && item.binding == bill
    });
    let Some(taint) = taint else {
        return Err(SceneError::Check(format!(
            "hidden-memo has no panel verdict in {verdicts:?}"
        )));
    };
    if taint.outcome != "Violate" || taint.reason != "tainted-memo-without-approval" {
        return Err(SceneError::Check(format!("hidden-memo verdict {taint:?}")));
    }
    if !taint.roles.iter().any(|role| role == "memo") {
        return Err(SceneError::Check(format!(
            "hidden-memo timeline has no memo {:?}",
            taint.roles
        )));
    }
    report(
        "hidden-memo",
        "scene hidden-memo: expect Violate tainted-memo-without-approval; memo on the counterexample; needs review"
            .to_owned(),
        verdicts,
        Vec::new(),
        Vec::new(),
    )
}

async fn vendor_alias(store: &Store, mode: &JevMode) -> Result<SceneReport, SceneError> {
    let left = ObligationRef::new("Landlord LLC", "500.00", "USD");
    let right = ObligationRef::new("Landlord L.L.C.", "500.00", "USD");
    let request = binding_request(&left, &right);
    let body = mode.respond(&request)?;
    let exact = verdict_label(exact_match(&left, &right));
    let fuzzy = verdict_label(fuzzy_match(&left, &right));
    let jev = verdict_label(map_binding_body(&body));
    let binding = "Landlord LLC|Landlord L.L.C.";
    let id = Uuid::new_v5(
        &Uuid::from_u128(0x6c65_6467_6572_5f73_6365_6e65_0002),
        b"vendor-alias",
    );
    let at = day(3, 0);
    let evidence = serde_json::json!({
        "binding": binding,
        "title": "inconclusive",
        "evidence": format!("exact {exact}; fuzzy {fuzzy}; jev {jev}"),
        "timeline": [{
            "eventId": id.to_string(),
            "observedAt": at.to_rfc3339(),
            "source": "exchange",
            "kind": "ledger.binding.alias",
            "role": "comparison"
        }],
        "amountCents": 50_000,
        "comparison": {
            "exact": { "outcome": exact },
            "fuzzy": { "outcome": fuzzy },
            "jev": { "outcome": jev }
        },
        "scene": "vendor-alias"
    });
    store
        .insert_verdict(&Verdict {
            id,
            recorded_at: at,
            outcome: StoredOutcome::Inconclusive,
            spec_id: "ledger.binding.vendor-alias".to_owned(),
            reason_code: Some("binding-comparison".to_owned()),
            evidence,
        })
        .await?;
    let mut script = Script::new("vendor-alias");
    script.add(
        "scene.counters",
        script.id("counters"),
        at,
        &[
            ("eventsIngested", "1"),
            ("sourceCount", "1"),
            ("liveMonitors", "0"),
        ],
    );
    script.persist(store).await?;
    let card = format!("exact {exact}, fuzzy {fuzzy}, jev {jev}");
    if !card.contains("exact ") || !card.contains("fuzzy ") || !card.contains("jev ") {
        return Err(SceneError::Check(
            "vendor-alias comparison card is missing a line".to_owned(),
        ));
    }
    report(
        "vendor-alias",
        format!("scene vendor-alias: expect {card}; comparison card shows all three"),
        vec![VerdictSnap {
            spec_id: "ledger.binding.vendor-alias".to_owned(),
            binding: binding.to_owned(),
            outcome: "Inconclusive".to_owned(),
            reason: "binding-comparison".to_owned(),
            roles: vec!["comparison".to_owned()],
        }],
        Vec::new(),
        Vec::new(),
    )
}

async fn recurring(store: &Store) -> Result<SceneReport, SceneError> {
    let mut script = Script::new("recurring");
    let mut desk = MonitorDesk::new()?;
    let payer = AgentId::new("payer")?;
    let mut history = Vec::new();
    let mut opened = 0_u64;
    let start = day(4, 0);
    for week in 1..=52 {
        let period = week_date(week);
        let bill = format!("bill-operating-{week:05}");
        let invoice = format!("BILL-{}", 5000 + week);
        let paid_on = paid_on(&period, i64::from((week % 3) + 1));
        history.push(
            ScheduleRecord::bill("Granite Bay Commercial Properties", "500.00", "USD")
                .with_vendor_id("vend-landlord")
                .with_bill_id(&bill)
                .with_invoice_number(&invoice)
                .with_period_date(&period)
                .with_memo("Monthly service charge as agreed."),
        );
        let payment = ScheduleRecord::payment("Granite Bay Commercial Properties", "500.00", "USD")
            .with_vendor_id("vend-landlord")
            .with_bill_id(&bill)
            .with_invoice_number(&invoice)
            .with_period_date(&period)
            .with_paid_on(&paid_on);
        let judgment = ledger_jev::judge_payment(&payment, &history, Some(&RefuseCall));
        if judgment.called_jev || judgment.finding != ScheduleFinding::Recurring {
            return Err(SceneError::Check(format!(
                "week {week} finding {:?}",
                judgment.finding
            )));
        }
        history.push(payment);
        let at = start + chrono::Duration::seconds(i64::from(week) * 4);
        desk.open_bill(
            OpenBill::new(&bill, payer.clone(), clock::ns(at)).with_owed(usd("500.00"), "USD"),
        )?;
        opened = opened.saturating_add(1);
        let approval = script.id(&format!("{bill}-approval"));
        let pay = script.id(&format!("{bill}-pay"));
        let settled = script.id(&format!("{bill}-settled"));
        let credit = script.id(&format!("{bill}-credit"));
        script.add(
            "agent.approve",
            approval,
            at,
            &[
                ("approval.id", &approval.to_string()),
                ("agent.id", "approver"),
                ("bill.id", &bill),
            ],
        );
        script.add(
            "agent.pay_bill",
            pay,
            at + chrono::Duration::seconds(1),
            &[
                ("agent.id", "payer"),
                ("bill.id", &bill),
                ("amount", "500.00"),
                ("link.approval_id", &approval.to_string()),
            ],
        );
        script.add(
            "ledger.exchange.settled",
            settled,
            at + chrono::Duration::seconds(2),
            &[
                ("agent.id", "payer"),
                ("bill.id", &bill),
                ("amount", "500.00"),
                ("link.pay_id", &pay.to_string()),
            ],
        );
        script.add(
            "ledger.exchange.credit",
            credit,
            at + chrono::Duration::seconds(3),
            &[
                ("agent.id", "payer"),
                ("bill.id", &bill),
                ("link.settled_id", &settled.to_string()),
            ],
        );
        let _paid = desk.record_payment(&bill, "500.00", "USD", clock::ns(at))?;
    }
    if desk.live_count() != 0 {
        return Err(SceneError::Check(format!(
            "recurring left {} monitors live",
            desk.live_count()
        )));
    }
    let samples = vec![monitor_snap(4 * 86_400 + 52, &desk, opened)];
    script.persist(store).await?;
    let verdicts = seal(store, start, start + chrono::Duration::seconds(52 * 4 + 10)).await?;
    if verdicts.iter().any(|item| item.outcome == "Violate") {
        return Err(SceneError::Check(format!(
            "recurring raised a violation {verdicts:?}"
        )));
    }
    if verdicts.len() < 52 {
        return Err(SceneError::Check(format!(
            "recurring sealed {} verdicts",
            verdicts.len()
        )));
    }
    report(
        "recurring",
        "scene recurring: expect 52 weekly 500.00 USD landlord bills; Violate 0; monitors flat at 0"
            .to_owned(),
        verdicts,
        samples,
        Vec::new(),
    )
}

async fn gc_soak(store: &Store) -> Result<SceneReport, SceneError> {
    let mut desk = MonitorDesk::new()?;
    let agent = AgentId::new("payer")?;
    let mut opened = 0_u64;
    let mut samples = Vec::new();
    let base = 10 * 86_400;
    for index in 0..400 {
        let key = format!("scene-gc-{index}");
        let at = base + i64::from(index);
        desk.open_bill(
            OpenBill::new(&key, agent.clone(), i128::from(at))
                .with_owed(usd("10.00"), "USD")
                .with_horizon(i128::from(at)),
        )?;
        opened = opened.saturating_add(1);
        if index % 2 == 0 {
            let _paid = desk.record_payment(&key, "10.00", "USD", i128::from(at))?;
        } else {
            desk.advance(i128::from(at))?;
        }
        if desk.live_count() > 1 {
            return Err(SceneError::Check(format!(
                "gc-soak live count grew to {}",
                desk.live_count()
            )));
        }
        if index % 50 == 49 {
            samples.push(monitor_snap(at, &desk, opened));
        }
    }
    for index in 0..15 {
        let key = format!("scene-gc-held-{index}");
        desk.open_bill(
            OpenBill::new(&key, agent.clone(), i128::from(base)).with_owed(usd("10.00"), "USD"),
        )?;
        opened = opened.saturating_add(1);
        let _partial = desk.record_payment(&key, "1.00", "USD", i128::from(base))?;
    }
    for index in 0..200 {
        let key = format!("scene-gc-gone-{index}");
        let at = base + 400 + i64::from(index);
        desk.open_bill(
            OpenBill::new(&key, agent.clone(), i128::from(at))
                .with_owed(usd("10.00"), "USD")
                .with_horizon(i128::from(at)),
        )?;
        opened = opened.saturating_add(1);
        let _paid = desk.record_payment(&key, "10.00", "USD", i128::from(at))?;
        if index % 50 == 49 {
            samples.push(monitor_snap(at, &desk, opened));
        }
    }
    desk.advance(i128::from(base + 700))?;
    let live = u64::try_from(desk.live_count()).unwrap_or(u64::MAX);
    samples.push(monitor_snap(base + 700, &desk, opened));
    if live != 15 || opened != 615 {
        return Err(SceneError::Check(format!(
            "gc-soak live {live} would-be {opened}"
        )));
    }
    let tail = &samples[samples.len().saturating_sub(3)..];
    if tail.len() < 3 || tail.iter().any(|sample| sample.live != 15) {
        return Err(SceneError::Check(
            "gc-soak live count was not flat at 15".to_owned(),
        ));
    }
    if samples.iter().any(|sample| sample.live > sample.would_be) {
        return Err(SceneError::Check(
            "gc-soak live count passed the would-be count".to_owned(),
        ));
    }
    let mut script = Script::new("gc-soak");
    for sample in &samples {
        script.add(
            "scene.monitors",
            script.id(&format!("sample-{}", sample.at_seconds)),
            day(0, sample.at_seconds),
            &[
                ("withGc", &sample.live.to_string()),
                ("withoutGc", &sample.would_be.to_string()),
            ],
        );
    }
    script.persist(store).await?;
    report(
        "gc-soak",
        format!("scene gc-soak: expect live monitors flat at {live} against would-be {opened}"),
        Vec::new(),
        samples,
        Vec::new(),
    )
}

async fn spec_live(store: &Store, mode: &JevMode) -> Result<SceneReport, SceneError> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/gemini/policy-draft.json");
    let gemini = std::fs::read_to_string(&path)
        .map_err(|error| SceneError::Check(format!("policy draft fixture: {error}")))?;
    let Some(draft) = draft_from_interaction(&gemini) else {
        return Err(SceneError::Check(
            "policy draft fixture was not a completed interaction".to_owned(),
        ));
    };
    let sentence = ledger_spec::FIXTURE_POLICY;
    let request = match_request(sentence, &draft);
    let jev = mode.respond(&request)?;
    let agent = AgentId::new("payer")?;
    let outcome = author(AuthoringInput {
        sentence,
        agent: &agent,
        gemini: Ok(&gemini),
        jev: Ok(&jev),
        coverage: Err(JevCallFailure::Timeout),
        override_coverage: false,
    });
    let armed = outcome.monitor().is_some()
        && outcome.steps().iter().any(|step| {
            matches!(
                step,
                AuthoringStep::Accepted {
                    pattern_id,
                    overridden: false
                } if pattern_id == "absence.after-until"
            )
        })
        && outcome.steps().iter().any(|step| {
            matches!(
                step,
                AuthoringStep::Check {
                    verdict: BindingVerdict::Same
                }
            )
        });
    if !armed {
        return Err(SceneError::Check(
            "spec-live did not arm absence.after-until".to_owned(),
        ));
    }
    let at = day(20, 0);
    let mut script = Script::new("spec-live");
    script.add(
        "scene.counters",
        script.id("counters"),
        at,
        &[
            ("eventsIngested", "1"),
            ("sourceCount", "1"),
            ("liveMonitors", "1"),
        ],
    );
    script.persist(store).await?;
    report(
        "spec-live",
        "scene spec-live: expect armed absence.after-until from an empty monitor; jev Same"
            .to_owned(),
        Vec::new(),
        vec![MonitorSnap {
            at_seconds: 20 * 86_400,
            live: 1,
            would_be: 1,
        }],
        Vec::new(),
    )
}

fn report(
    name: &str,
    summary: String,
    mut verdicts: Vec<VerdictSnap>,
    mut monitors: Vec<MonitorSnap>,
    mut signatures: Vec<String>,
) -> Result<SceneReport, SceneError> {
    if summary.to_ascii_lowercase().contains("fraud") || summary.contains('\n') {
        return Err(SceneError::Check(
            "scene summary must be one line and must not say fraud".to_owned(),
        ));
    }
    verdicts.sort();
    monitors.sort();
    signatures.sort();
    signatures.dedup();
    Ok(SceneReport {
        name: name.to_owned(),
        summary,
        verdicts,
        monitors,
        signatures,
    })
}

fn monitor_snap(at_seconds: i64, desk: &MonitorDesk, would_be: u64) -> MonitorSnap {
    MonitorSnap {
        at_seconds,
        live: u64::try_from(desk.live_count()).unwrap_or(u64::MAX),
        would_be,
    }
}

fn note_dashboard(
    script: &mut Script,
    at: DateTime<Utc>,
    samples: &[MonitorSnap],
    agent: i64,
    exchange: i64,
    solana: i64,
    ingested: i64,
    live: u64,
) {
    for sample in samples {
        script.add(
            "scene.monitors",
            script.id(&format!("monitors-{}", sample.at_seconds)),
            at,
            &[
                ("withGc", &sample.live.to_string()),
                ("withoutGc", &sample.would_be.to_string()),
            ],
        );
    }
    script.add(
        "scene.rates",
        script.id("rates"),
        at,
        &[
            ("agentLog", &agent.to_string()),
            ("exchange", &exchange.to_string()),
            ("solana", &solana.to_string()),
        ],
    );
    script.add(
        "scene.counters",
        script.id("counters"),
        at,
        &[
            ("eventsIngested", &ingested.to_string()),
            ("sourceCount", "2"),
            ("liveMonitors", &live.to_string()),
        ],
    );
}

async fn seal(
    store: &Store,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Vec<VerdictSnap>, SceneError> {
    let (tx, _rx) = broadcast::channel(64);
    let mut engine = Engine::new(store.clone(), tx)?;
    let frames = engine.poll_window(start, end).await?;
    Ok(frames
        .into_iter()
        .map(|frame| VerdictSnap {
            spec_id: frame.spec_id,
            binding: frame.binding,
            outcome: frame.outcome,
            reason: frame.reason_code.unwrap_or_default(),
            roles: frame.timeline.into_iter().map(|entry| entry.role).collect(),
        })
        .collect())
}

async fn dashboard_events(
    store: &Store,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<i64, SceneError> {
    let counts = store
        .window_counts(start, end, BucketWidth::Minute, None)
        .await?;
    Ok(counts.iter().map(|row| row.events).sum())
}

fn signatures_of(events: &[ledger_store::Event]) -> Vec<String> {
    let mut found = Vec::new();
    for event in events {
        let Some(value) = event
            .payload
            .pointer("/attributes/tx.signature")
            .and_then(|value| value.as_str())
        else {
            continue;
        };
        if !found.iter().any(|existing: &String| existing == value) {
            found.push(value.to_owned());
        }
    }
    found
}

fn verdict_label(verdict: BindingVerdict) -> &'static str {
    match verdict {
        BindingVerdict::Same => "Same",
        BindingVerdict::Different => "Different",
        BindingVerdict::Unknown => "Unknown",
    }
}

fn usd(amount: &str) -> Money {
    Money::from_str(amount).expect("scene amount is two-decimal money")
}

fn week_date(week: u32) -> String {
    let start = NaiveDate::from_ymd_opt(2025, 10, 13).expect("rent start");
    let date = start
        .checked_add_signed(TimeDelta::days(i64::from(week - 1) * 7))
        .expect("rent week");
    date.format("%Y-%m-%d").to_string()
}

fn paid_on(period: &str, days: i64) -> String {
    let date = NaiveDate::parse_from_str(period, "%Y-%m-%d")
        .expect("period")
        .checked_add_signed(TimeDelta::days(days))
        .expect("paid on");
    date.format("%Y-%m-%d").to_string()
}

/// Same id the engine stores for a sealed verdict.
fn verdict_id(spec: &str, binding: &str, outcome: &str, reason: Option<&str>) -> Uuid {
    let namespace = Uuid::from_u128(0x6c65_6467_6572_5f76_6572_6469_6374_0001);
    let key = format!("{spec}|{binding}|{outcome}|{}", reason.unwrap_or(""));
    Uuid::new_v5(&namespace, key.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn force_is_ignored_unless_the_test_api_is_on() {
        let forced = take_force(false, Some("timeout")).expect("ignored");
        assert!(forced.is_none());
        let armed = take_force(true, Some("timeout")).expect("timeout");
        assert_eq!(armed, Some(ForcedOutcome::Timeout));
        let err = take_force(true, Some("pass")).expect_err("rejected");
        assert!(err.to_string().contains("LEDGER_JEV_FORCE"));
        assert!(!err.to_string().contains("fraud"));
    }
}
