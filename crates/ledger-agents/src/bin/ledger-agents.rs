//! Runs the company-001 buying desk and accepts live fault toggles.
//!
//! `ledger-agents double-pay no-approval` turns those faults on for the first
//! cycle, then listens for `toggle <fault>` on `.ledger/agents.sock`.
//! `ledger-agents --toggle double-pay` flips that fault on the running desk.

use std::path::Path;
use std::str::FromStr;

use ledger_agents::{listen, toggle_running, AgentError, Desk, Fault, FaultConfig, CONTROL_SOCKET};
use ledger_core::dataset::{load_company, Money};
use ledger_exchange::{install_telemetry, TelemetryError, TelemetryGuard};

fn main() {
    let code = match run() {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("agents: {err}");
            1
        }
    };
    std::process::exit(code);
}

fn run() -> Result<(), AgentError> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "--toggle") {
        if args.len() != 2 {
            return Err(AgentError::Usage);
        }
        let fault = Fault::parse(&args[1])?;
        let enabled = toggle_running(Path::new(CONTROL_SOCKET), fault)?;
        println!("{}", if enabled { "on" } else { "off" });
        return Ok(());
    }

    let _telemetry = install_tracing()?;
    let faults = FaultConfig::from_names(args.iter().map(String::as_str))?;
    let mut desk = Desk::new(load_books()?, budget()?, faults)?;
    let report = desk.run_cycle()?;
    print_report(&report);
    println!("fault control listening on {CONTROL_SOCKET}");
    listen(&mut desk, Path::new(CONTROL_SOCKET))
}

fn install_tracing() -> Result<Option<TelemetryGuard>, AgentError> {
    match install_telemetry() {
        Ok(guard) => Ok(Some(guard)),
        Err(TelemetryError::MissingDsn) => {
            let _ = tracing_subscriber::fmt()
                .with_max_level(tracing::Level::INFO)
                .try_init();
            Ok(None)
        }
        Err(TelemetryError::AlreadyInstalled) => Ok(None),
        Err(_) => Err(AgentError::TelemetryDisabled),
    }
}

fn load_books() -> Result<ledger_core::dataset::CompanyBooks, AgentError> {
    let dir = Path::new("data/hackathon-100/company-001");
    if !dir.join("rwm-v4").is_dir() {
        return Err(AgentError::MissingDataset);
    }
    Ok(load_company(dir)?)
}

fn budget() -> Result<Money, AgentError> {
    match std::env::var("LEDGER_BUDGET") {
        Err(_) => Ok(default_budget()),
        Ok(value) if value.trim().is_empty() => Ok(default_budget()),
        Ok(value) => value.trim().parse().map_err(|_| AgentError::InvalidBudget),
    }
}

fn default_budget() -> Money {
    Money::from_str("5000.00").expect("5000.00 is two-decimal money")
}

fn print_report(report: &ledger_agents::CycleReport) {
    let review = report
        .payments
        .iter()
        .filter(|payment| payment.needs_review)
        .count();
    println!(
        "payments: {}, withheld: {}, needs review: {review}",
        report.payments.len(),
        report.withheld.len(),
    );
    for payment in &report.payments {
        if payment.needs_review {
            let faults = payment
                .faults
                .iter()
                .map(|fault| fault.as_str())
                .collect::<Vec<_>>()
                .join(",");
            println!("needs review: bill {} ({faults})", payment.bill_local);
        }
    }
    println!(
        "trade: order {} fill {}",
        report.trade.order_id, report.trade.fill_id
    );
}
