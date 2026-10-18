//! Background writer that batches closed spans into Timescale.
//!
//! The worker is the ingest half of dual export. The process subscriber fans
//! the same spans here and to Sentry. This half has no sample rate.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::capture::{CapturedSpan, IngestLayer, SpanSink};
use crate::error::IngestError;
use crate::persist::persist;
use ledger_store::{database_url, Store};

const BATCH_WAIT: Duration = Duration::from_millis(50);
const BATCH_LIMIT: usize = 64;

enum Note {
    Span(CapturedSpan),
    Flush(Sender<Result<(), IngestError>>),
    Shutdown,
}

struct NoteSink {
    tx: Sender<Note>,
}

impl SpanSink for NoteSink {
    fn on_span(&self, span: CapturedSpan) {
        let _ = self.tx.send(Note::Span(span));
    }
}

/// Owns the thread that writes closed spans to Timescale.
#[must_use = "dropping the worker stops ingest"]
pub struct IngestWorker {
    notes: Sender<Note>,
    thread: Option<JoinHandle<()>>,
}

impl IngestWorker {
    /// Connects to Timescale and returns the layer that feeds the worker.
    ///
    /// The layer forwards every span it receives. It does not sample.
    ///
    /// # Errors
    ///
    /// Returns [`IngestError::Startup`] when the thread or runtime cannot
    /// start, [`IngestError::Store`] when Timescale cannot be reached, and
    /// [`IngestError::WorkerStopped`] when the worker exits before it is ready.
    pub fn start() -> Result<(IngestLayer, Self), IngestError> {
        let (notes, incoming) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        let url = database_url();
        let thread = std::thread::Builder::new()
            .name("ledger-ingest".to_owned())
            .spawn(move || run_worker(url, incoming, ready_tx))
            .map_err(|error| IngestError::Startup(error.to_string()))?;
        let ready = match ready_rx.recv() {
            Ok(ready) => ready,
            Err(_) => {
                let _ = thread.join();
                return Err(IngestError::WorkerStopped);
            }
        };
        if let Err(error) = ready {
            let _ = thread.join();
            return Err(error);
        }
        let layer = IngestLayer::from_sink(NoteSink { tx: notes.clone() });
        Ok((
            layer,
            Self {
                notes,
                thread: Some(thread),
            },
        ))
    }

    /// Writes spans already queued, then stops the worker.
    ///
    /// # Errors
    ///
    /// Returns [`IngestError::WorkerStopped`] when the worker has already
    /// exited, and [`IngestError::Store`] when a queued insert failed.
    pub fn close(mut self) -> Result<(), IngestError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.notes
            .send(Note::Flush(reply_tx))
            .map_err(|_| IngestError::WorkerStopped)?;
        let result = reply_rx.recv().map_err(|_| IngestError::WorkerStopped)?;
        let _ = self.notes.send(Note::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        result
    }
}

impl Drop for IngestWorker {
    fn drop(&mut self) {
        if self.thread.is_some() {
            let _ = self.notes.send(Note::Shutdown);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }
}

fn run_worker(url: String, incoming: Receiver<Note>, ready: Sender<Result<(), IngestError>>) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            let _ = ready.send(Err(IngestError::Startup(error.to_string())));
            return;
        }
    };
    let store = match runtime.block_on(Store::connect(&url)) {
        Ok(store) => store,
        Err(error) => {
            let _ = ready.send(Err(IngestError::Store(error)));
            return;
        }
    };
    let _ = ready.send(Ok(()));

    let mut batch = Vec::new();
    let mut failed: Option<IngestError> = None;
    loop {
        let note = if batch.is_empty() {
            match incoming.recv() {
                Ok(note) => note,
                Err(_) => break,
            }
        } else {
            match incoming.recv_timeout(BATCH_WAIT) {
                Ok(note) => note,
                Err(RecvTimeoutError::Timeout) => {
                    remember(&mut failed, flush(&runtime, &store, &mut batch));
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => {
                    remember(&mut failed, flush(&runtime, &store, &mut batch));
                    break;
                }
            }
        };
        match note {
            Note::Span(span) => {
                batch.push(span);
                if batch.len() >= BATCH_LIMIT {
                    remember(&mut failed, flush(&runtime, &store, &mut batch));
                }
            }
            Note::Flush(reply) => {
                remember(&mut failed, flush(&runtime, &store, &mut batch));
                let result = match failed.take() {
                    Some(error) => Err(error),
                    None => Ok(()),
                };
                let _ = reply.send(result);
            }
            Note::Shutdown => {
                remember(&mut failed, flush(&runtime, &store, &mut batch));
                break;
            }
        }
    }
}

fn flush(
    runtime: &tokio::runtime::Runtime,
    store: &Store,
    batch: &mut Vec<CapturedSpan>,
) -> Result<(), IngestError> {
    if batch.is_empty() {
        return Ok(());
    }
    let spans = std::mem::take(batch);
    runtime.block_on(persist(store, &spans)).map(|_| ())
}

fn remember(failed: &mut Option<IngestError>, result: Result<(), IngestError>) {
    if failed.is_none() {
        if let Err(error) = result {
            *failed = Some(error);
        }
    }
}
