//! Line protocol on a Unix socket so `just fault <name>` can flip a running desk.
//!
//! One line `toggle <fault>` flips that fault, runs another Rig cycle, and
//! replies `on` or `off`.

use std::fs;
use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader as AsyncBufReader};
use tokio::net::UnixListener;

use crate::desk::Desk;
use crate::error::AgentError;
use crate::fault::Fault;
use crate::report::CycleReport;

/// Socket path relative to the process working directory. `just` runs from the repo root.
pub const CONTROL_SOCKET: &str = ".ledger/agents.sock";

#[derive(Debug)]
struct ToggleOutcome {
    enabled: bool,
    report: CycleReport,
}

/// Accepts `toggle <fault>` lines until the process is stopped.
///
/// # Errors
///
/// Returns [`AgentError::AlreadyRunning`] when another desk holds `path`,
/// and [`AgentError::ControlBind`] when the socket cannot be created.
pub async fn listen(desk: &Desk, path: &Path) -> Result<(), AgentError> {
    let listener = bind_async(path)?;
    let _cleanup = RemoveOnDrop(path.to_owned());
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                if let Err(err) = handle(desk, stream).await {
                    tracing::warn!(error = %err, "fault control");
                }
            }
            Err(err) => {
                tracing::warn!(error = %err, "fault control");
            }
        }
    }
}

/// Tells the desk listening on `path` to flip `fault` and run a cycle.
///
/// # Errors
///
/// Returns [`AgentError::NotRunning`] when nothing is listening, and
/// [`AgentError::Control`] when the desk rejects the command.
pub fn toggle_running(path: &Path, fault: Fault) -> Result<bool, AgentError> {
    let mut stream = UnixStream::connect(path).map_err(not_running)?;
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .map_err(AgentError::ControlIo)?;
    writeln!(stream, "toggle {fault}").map_err(AgentError::ControlIo)?;
    stream
        .shutdown(Shutdown::Write)
        .map_err(AgentError::ControlIo)?;
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(AgentError::ControlIo)?;
    match response.trim() {
        "on" => Ok(true),
        "off" => Ok(false),
        other => {
            let detail = other.strip_prefix("error ").unwrap_or(other);
            Err(AgentError::Control(detail.to_owned()))
        }
    }
}

async fn apply_line(desk: &Desk, line: &str) -> Result<ToggleOutcome, AgentError> {
    let line = line.trim();
    let Some(name) = line.strip_prefix("toggle ") else {
        return Err(AgentError::Control("expected toggle <fault>".to_owned()));
    };
    let fault = Fault::parse(name.trim())?;
    let enabled = desk.toggle_fault(fault);
    let report = desk.run_cycle().await?;
    tracing::info!(fault = fault.as_str(), enabled, "fault toggled");
    Ok(ToggleOutcome { enabled, report })
}

async fn handle(desk: &Desk, stream: tokio::net::UnixStream) -> Result<(), AgentError> {
    let mut reader = AsyncBufReader::new(stream);
    let mut line = String::new();
    let read = tokio::time::timeout(Duration::from_secs(5), reader.read_line(&mut line))
        .await
        .map_err(|_| AgentError::Control("fault control timed out".to_owned()))?
        .map_err(AgentError::ControlIo)?;
    if read == 0 && line.is_empty() {
        return Ok(());
    }
    let mut stream = reader.into_inner();
    let response = match apply_line(desk, &line).await {
        Ok(outcome) => {
            let needs_review = outcome
                .report
                .payments
                .iter()
                .filter(|payment| payment.needs_review)
                .count();
            tracing::info!(
                payments = outcome.report.payments.len(),
                needs_review,
                "fault cycle finished"
            );
            if outcome.enabled {
                "on\n".to_owned()
            } else {
                "off\n".to_owned()
            }
        }
        Err(err) => format!("error {err}\n"),
    };
    stream
        .write_all(response.as_bytes())
        .await
        .map_err(AgentError::ControlIo)?;
    stream.shutdown().await.map_err(AgentError::ControlIo)?;
    Ok(())
}

fn bind_async(path: &Path) -> Result<UnixListener, AgentError> {
    let listener = bind(path)?;
    listener
        .set_nonblocking(true)
        .map_err(AgentError::ControlBind)?;
    UnixListener::from_std(listener).map_err(AgentError::ControlBind)
}

fn bind(path: &Path) -> Result<std::os::unix::net::UnixListener, AgentError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(AgentError::ControlBind)?;
        }
    }
    if path.exists() {
        match UnixStream::connect(path) {
            Ok(_live) => return Err(AgentError::AlreadyRunning),
            Err(_) => fs::remove_file(path).map_err(AgentError::ControlBind)?,
        }
    }
    std::os::unix::net::UnixListener::bind(path).map_err(AgentError::ControlBind)
}

fn not_running(err: std::io::Error) -> AgentError {
    match err.kind() {
        std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused => {
            AgentError::NotRunning
        }
        _ => AgentError::ControlIo(err),
    }
}

struct RemoveOnDrop(PathBuf);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;

    use ledger_core::dataset::{load_company, CompanyBooks};

    use crate::fault::FaultConfig;
    use crate::replay;

    static SOCKET_SEQ: AtomicU64 = AtomicU64::new(0);

    #[tokio::test(flavor = "current_thread")]
    async fn toggle_line_enables_the_fault_for_the_next_payment() {
        let desk = fixture_desk();
        let outcome = apply_line(&desk, "toggle double-pay\n")
            .await
            .expect("toggle");
        assert!(outcome.enabled);
        assert!(desk.fault_enabled(Fault::DoublePay));
        let _ = desk.approve_bill("bill-settled");
        let paid = desk.pay_bill("bill-settled");
        let crate::PayOutcome::Paid(payment) = paid else {
            panic!("expected a second payment, got {paid:?}");
        };
        assert!(payment.faults.contains(&Fault::DoublePay));
        assert!(payment.needs_review);
        assert!(!format!("{payment:?}")
            .to_ascii_lowercase()
            .contains("fraud"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn unknown_toggle_does_not_run_a_cycle() {
        let desk = fixture_desk();
        let error = apply_line(&desk, "toggle fraud")
            .await
            .expect_err("unknown");
        assert!(matches!(error, AgentError::UnknownFault { .. }));
        assert!(desk.report().payments.is_empty());
    }

    #[test]
    fn socket_toggle_flips_the_running_desk() {
        let path = temp_socket();
        let listener = bind(&path).expect("bind");
        listener.set_nonblocking(true).expect("nonblocking");
        let desk = fixture_desk();
        let server = thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime");
            runtime.block_on(async move {
                let listener = UnixListener::from_std(listener).expect("tokio listener");
                serve_one(&desk, &listener).await.expect("first toggle");
                serve_one(&desk, &listener).await.expect("second toggle");
            });
        });

        assert!(toggle_running(&path, Fault::DoublePay).expect("on"));
        assert!(!toggle_running(&path, Fault::DoublePay).expect("off"));
        server.join().expect("server");
        let _ = fs::remove_file(&path);
    }

    async fn serve_one(desk: &Desk, listener: &UnixListener) -> Result<(), AgentError> {
        let (stream, _) = listener.accept().await.map_err(AgentError::ControlIo)?;
        handle(desk, stream).await
    }

    fn fixture_desk() -> Desk {
        let books = load_fixture("company-fixture");
        let desk = Desk::new(books, money("150.00"), FaultConfig::new()).expect("desk");
        desk.replay_gemini(replay::serve());
        desk
    }

    fn load_fixture(name: &str) -> CompanyBooks {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join(name);
        load_company(&path).unwrap_or_else(|err| panic!("fixture {name}: {err}"))
    }

    fn money(raw: &str) -> ledger_core::dataset::Money {
        raw.parse().unwrap_or_else(|_| panic!("money {raw}"))
    }

    fn temp_socket() -> PathBuf {
        let seq = SOCKET_SEQ.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("ledger-agents-{}-{seq}", std::process::id()))
    }
}
