//! Failures from fault configuration, bill payment, and the control socket.

use ledger_core::dataset::LoadError;
use ledger_exchange::ExchangeError;
use thiserror::Error;

/// Why a buying-agent run could not continue.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum AgentError {
    /// The fault name is not one of the five injectable faults.
    #[error(
        "unknown fault {name}; expected double-pay, no-approval, exceed-budget, swapped-vendor, or hidden-memo"
    )]
    UnknownFault {
        /// The name that was not recognized.
        name: String,
    },

    /// The process was started with `--toggle` and no fault name.
    #[error("usage: ledger-agents [--toggle <fault>] [fault...]")]
    Usage,

    /// A budget below zero is not a limit.
    #[error("negative budget")]
    NegativeBudget,

    /// `LEDGER_BUDGET` was set and was not a two-decimal amount.
    #[error("invalid budget")]
    InvalidBudget,

    /// `just data` has not produced company-001 in this checkout.
    #[error("company-001 dataset is not present; run `just data`")]
    MissingDataset,

    /// The buyer's order did not trade. The resting ask should have crossed it.
    #[error("buyer order did not fill")]
    Unfilled,

    /// The in-process order id counter overflowed `u64`.
    #[error("order id overflowed")]
    OrderIdOverflow,

    /// A desk is already bound to the fault-control socket.
    #[error("agents are already running")]
    AlreadyRunning,

    /// Nothing is listening on the fault-control socket.
    #[error("agents are not running")]
    NotRunning,

    /// The control socket accepted a command it could not apply.
    #[error("fault control: {0}")]
    Control(String),

    /// The control socket could not be created.
    #[error("failed to bind the fault control socket")]
    ControlBind(#[source] std::io::Error),

    /// A read or write on the control socket failed.
    #[error("failed to use the fault control socket")]
    ControlIo(#[source] std::io::Error),

    /// `SENTRY_DSN` was set, but the Sentry client did not start.
    ///
    /// The DSN value is never included in this error.
    #[error("sentry client is disabled")]
    TelemetryDisabled,

    /// Company YAML could not be loaded.
    #[error(transparent)]
    Dataset(#[from] LoadError),

    /// The exchange rejected an order or stopped on a book defect.
    #[error(transparent)]
    Exchange(#[from] ExchangeError),
}
