//! Per-bill absence-after: a second payment before a credit violates.

use ledger_core::{
    adapt_trace, judge_bill_absence, AgentId, BillId, DataClass, InconclusiveReason, Integrity,
    LedgerEvent, LedgerKind, SourceCapability, SourceTier, Times, VerdictAuthority, VerdictOutcome,
    WithheldSource,
};

fn agent() -> AgentId {
    AgentId::new("buyer-1").expect("agent id")
}

fn bill(id: &str) -> BillId {
    BillId::new(id).expect("bill id")
}

fn event(
    id: &str,
    kind: LedgerKind,
    bill_id: &str,
    at: i128,
    predecessors: &[&str],
) -> LedgerEvent {
    LedgerEvent::new(
        id,
        kind,
        bill(bill_id),
        agent(),
        Times::at(at),
        predecessors.iter().copied(),
    )
    .expect("ledger event")
}

fn clean(bill_id: &str, prefix: &str) -> Vec<LedgerEvent> {
    let pay = format!("{prefix}-pay");
    let settled = format!("{prefix}-settled");
    let credit = format!("{prefix}-credit");
    vec![
        event(&pay, LedgerKind::AgentPayBill, bill_id, 10, &[]),
        event(
            &settled,
            LedgerKind::ObligationSettled,
            bill_id,
            20,
            &[pay.as_str()],
        ),
        event(
            &credit,
            LedgerKind::Credit,
            bill_id,
            30,
            &[settled.as_str()],
        ),
    ]
}

fn double_pay(bill_id: &str, prefix: &str) -> Vec<LedgerEvent> {
    let pay = format!("{prefix}-pay");
    let settled = format!("{prefix}-settled");
    let pay_again = format!("{prefix}-pay-again");
    let extra = format!("{prefix}-extra");
    let credit = format!("{prefix}-credit");
    vec![
        event(&pay, LedgerKind::AgentPayBill, bill_id, 10, &[]),
        event(
            &settled,
            LedgerKind::ObligationSettled,
            bill_id,
            20,
            &[pay.as_str()],
        ),
        event(
            &pay_again,
            LedgerKind::AgentPayBill,
            bill_id,
            30,
            &[settled.as_str()],
        ),
        event(
            &extra,
            LedgerKind::ExtraPayment,
            bill_id,
            40,
            &[pay_again.as_str()],
        ),
        event(&credit, LedgerKind::Credit, bill_id, 50, &[extra.as_str()]),
    ]
}

#[test]
fn double_payment_before_credit_violates() {
    let verdicts = judge_bill_absence(&agent(), &double_pay("bill-1", "b"), None).expect("judge");
    assert_eq!(verdicts.len(), 1, "{verdicts:?}");
    assert_eq!(verdicts[0].binding(), "bill-1");
    assert_eq!(verdicts[0].verdict().outcome(), VerdictOutcome::Violate);
    assert_eq!(
        verdicts[0].verdict().authority(),
        VerdictAuthority::Authoritative
    );
}

#[test]
fn single_payment_then_credit_satisfies() {
    let verdicts = judge_bill_absence(&agent(), &clean("bill-1", "b"), None).expect("judge");
    assert_eq!(verdicts.len(), 1, "{verdicts:?}");
    assert_eq!(verdicts[0].binding(), "bill-1");
    assert_eq!(verdicts[0].verdict().outcome(), VerdictOutcome::Satisfy);
    assert_eq!(
        verdicts[0].verdict().authority(),
        VerdictAuthority::Authoritative
    );
}

#[test]
fn missing_exchange_evidence_is_inconclusive() {
    let events = clean("bill-1", "b");
    let adapted = adapt_trace(&agent(), &events, Some(WithheldSource::Exchange)).expect("adapt");
    assert!(adapted
        .evidence()
        .iter()
        .all(|evidence| evidence.source().source().as_str() != "exchange"));
    assert!(adapted
        .events()
        .iter()
        .any(|event| { event.producer().kind().as_str() == LedgerKind::Credit.event_kind() }));

    let verdicts =
        judge_bill_absence(&agent(), &events, Some(WithheldSource::Exchange)).expect("judge");
    assert_eq!(verdicts.len(), 1, "{verdicts:?}");
    assert_eq!(
        verdicts[0].verdict().outcome(),
        VerdictOutcome::Inconclusive
    );
    assert_ne!(verdicts[0].verdict().outcome(), VerdictOutcome::Satisfy);
    assert_eq!(
        verdicts[0].verdict().inconclusive_reason(),
        Some(InconclusiveReason::PrefixUncertainty)
    );
}

#[test]
fn two_bills_are_judged_separately() {
    let mut events = clean("bill-a", "a");
    events.extend(double_pay("bill-b", "b"));
    let verdicts = judge_bill_absence(&agent(), &events, None).expect("judge");
    assert_eq!(verdicts.len(), 2, "{verdicts:?}");
    assert_eq!(verdicts[0].binding(), "bill-a");
    assert_eq!(verdicts[0].verdict().outcome(), VerdictOutcome::Satisfy);
    assert_eq!(verdicts[1].binding(), "bill-b");
    assert_eq!(verdicts[1].verdict().outcome(), VerdictOutcome::Violate);
}

#[test]
fn adapter_fills_tier_assurance_observation_time_and_data_class() {
    let buyer = agent();
    let events = vec![
        LedgerEvent::new(
            "pay",
            LedgerKind::AgentPayBill,
            bill("bill-1"),
            buyer.clone(),
            Times::new(10, 42),
            std::iter::empty::<&str>(),
        )
        .expect("pay"),
        LedgerEvent::new(
            "credit",
            LedgerKind::Credit,
            bill("bill-1"),
            buyer.clone(),
            Times::new(20, 50),
            ["pay"],
        )
        .expect("credit"),
    ];
    let adapted = adapt_trace(&buyer, &events, None).expect("adapt");
    let pay = adapted
        .events()
        .iter()
        .find(|event| event.producer().kind().as_str() == LedgerKind::AgentPayBill.event_kind())
        .expect("pay event");
    let credit = adapted
        .events()
        .iter()
        .find(|event| event.producer().kind().as_str() == LedgerKind::Credit.event_kind())
        .expect("credit event");

    assert_eq!(pay.source().monitored_application().as_str(), "ledger");
    assert_eq!(pay.source().source().as_str(), "agent.buyer-1");
    assert_eq!(pay.source_tier(), SourceTier::Contributing);
    assert_eq!(credit.source().source().as_str(), "exchange");
    assert_eq!(credit.source_tier(), SourceTier::Anchor);
    assert_ne!(pay.source(), credit.source());
    assert_eq!(pay.data_class(), DataClass::Tenant);
    assert_eq!(credit.data_class(), DataClass::Tenant);
    assert_eq!(pay.integrity(), Integrity::Authenticated);
    assert_eq!(credit.integrity(), Integrity::Authenticated);
    assert_eq!(pay.observation_time().sentinel_nanoseconds(), 42);
    assert_eq!(
        pay.observation_time().sentinel_clock().as_str(),
        "ledger.sentinel"
    );
    assert_eq!(pay.occurrence_time().nanoseconds(), 10);
    assert!(pay.capabilities().contains(SourceCapability::CausalEdges));
    assert!(pay
        .capabilities()
        .contains(SourceCapability::CompletenessSignals));

    let exchange = adapted
        .evidence()
        .iter()
        .find(|evidence| evidence.source().source().as_str() == "exchange")
        .expect("exchange evidence");
    let tool_log = adapted
        .evidence()
        .iter()
        .find(|evidence| evidence.source().source().as_str() == "agent.buyer-1")
        .expect("agent evidence");
    assert_eq!(exchange.tier(), SourceTier::Anchor);
    assert_eq!(tool_log.tier(), SourceTier::Contributing);
}
