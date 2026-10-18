//! Line protocol on a Unix socket so `just fault <name>` can flip a running desk.
//!
//! One line `toggle <fault>` runs another decision cycle and replies `on` or `off`.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

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

/// Blocks, accepting `toggle <fault>` lines until the process is stopped.
///
/// # Errors
///
/// Returns [`AgentError::AlreadyRunning`] when another desk holds `path`,
/// and [`AgentError::ControlBind`] when the socket cannot be created.
pub fn listen(desk: &mut Desk, path: &Path) -> Result<(), AgentError> {
    let listener = bind(path)?;
    let _cleanup = RemoveOnDrop(path.to_owned());
    for connection in listener.incoming() {
        match connection {
            Ok(stream) => {
                if let Err(err) = handle(desk, stream) {
                    tracing::warn!(error = %err, "fault control");
                }
            }
            Err(err) => {
                tracing::warn!(error = %err, "fault control");
            }
        }
    }
    Ok(())
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
        .set_read_timeout(Some(Duration::from_secs(5)))
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

fn apply_line(desk: &mut Desk, line: &str) -> Result<ToggleOutcome, AgentError> {
    let line = line.trim();
    let Some(name) = line.strip_prefix("toggle ") else {
        return Err(AgentError::Control("expected toggle <fault>".to_owned()));
    };
    let fault = Fault::parse(name.trim())?;
    let enabled = desk.toggle_fault(fault);
    let report = desk.run_cycle()?;
    tracing::info!(fault = fault.as_str(), enabled, "fault toggled");
    Ok(ToggleOutcome { enabled, report })
}

fn handle(desk: &mut Desk, stream: UnixStream) -> Result<(), AgentError> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(AgentError::ControlIo)?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).map_err(AgentError::ControlIo)?;
    let mut stream = reader.into_inner();
    let response = match apply_line(desk, &line) {
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
                "on\n"
            } else {
                "off\n"
            }
        }
        Err(err) => {
            return write_response(&mut stream, &format!("error {err}\n"));
        }
    };
    write_response(&mut stream, response)
}

fn write_response(stream: &mut UnixStream, response: &str) -> Result<(), AgentError> {
    stream
        .write_all(response.as_bytes())
        .map_err(AgentError::ControlIo)?;
    stream
        .shutdown(Shutdown::Write)
        .map_err(AgentError::ControlIo)
}

fn bind(path: &Path) -> Result<UnixListener, AgentError> {
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
    UnixListener::bind(path).map_err(AgentError::ControlBind)
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

    static SOCKET_SEQ: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn toggle_line_runs_a_cycle_with_that_fault() {
        let mut desk = fixture_desk();
        let outcome = apply_line(&mut desk, "toggle double-pay\n").expect("toggle");
        assert!(outcome.enabled);
        assert!(outcome
            .report
            .payments
            .iter()
            .any(|payment| payment.faults.contains(&Fault::DoublePay)));
        assert!(outcome
            .report
            .payments
            .iter()
            .filter(|payment| payment.needs_review)
            .all(|payment| !format!("{payment:?}")
                .to_ascii_lowercase()
                .contains("fraud")));
    }

    #[test]
    fn unknown_toggle_does_not_run_a_cycle() {
        let mut desk = fixture_desk();
        let error = apply_line(&mut desk, "toggle fraud").expect_err("unknown");
        assert!(matches!(error, AgentError::UnknownFault { .. }));
    }

    #[test]
    fn socket_toggle_flips_the_running_desk() {
        let path = temp_socket();
        let listener = bind(&path).expect("bind");
        let mut desk = fixture_desk();
        let server = thread::spawn(move || {
            serve_one(&mut desk, &listener).expect("first toggle");
            serve_one(&mut desk, &listener).expect("second toggle");
        });

        assert!(toggle_running(&path, Fault::DoublePay).expect("on"));
        assert!(!toggle_running(&path, Fault::DoublePay).expect("off"));
        server.join().expect("server");
        let _ = fs::remove_file(&path);
    }

    fn serve_one(desk: &mut Desk, listener: &UnixListener) -> Result<(), AgentError> {
        let stream = listener.accept().map_err(AgentError::ControlIo)?.0;
        handle(desk, stream)
    }

    fn fixture_desk() -> Desk {
        let books = load_fixture("company-fixture");
        Desk::new(books, money("150.00"), FaultConfig::new()).expect("desk")
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
