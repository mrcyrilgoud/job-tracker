//! Run model for Jobs_Cycle and Posting_Check_Run executions.
//!
//! The `RunCoordinator` owns the run lifecycle, runner exclusivity, per-posting
//! scheduling, cancellation, retry, persistence, and progress publication. The
//! submodules are layered inward-out: pure model/lifecycle/ledger types first,
//! then the progress contract, SQLite store, coordinator, stages, and legacy
//! projections.

// Scaffolding: most items are consumed by later tasks (coordinator, commands,
// CLI adapters). Remove once the run model is wired into the command surface.
#![allow(dead_code)]

pub mod coordinator;
pub mod ledger;
pub mod legacy;
pub mod lifecycle;
pub mod model;
pub mod progress;
pub mod stages;
pub mod store;

#[cfg(test)]
pub(crate) mod test_support;

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use tokio::sync::watch;

/// In-process runs and their cancel signals, keyed by run id.
///
/// A run is registered at its accept point by `RunCoordinator::accept` and
/// unregistered when its [`RunRegistration`] drops (when the run's
/// `AcceptedRun` is dropped at the end of execution, or on any early exit).
/// Registration is what makes a snapshot `live`: this process owns the run and
/// will publish its events. Cross-process cancels do not use the registry;
/// they are committed to SQLite and picked up by the dispatch loop's poll.
#[derive(Clone, Default)]
pub struct RunRegistry {
    inner: Arc<Mutex<HashMap<String, watch::Sender<bool>>>>,
}

impl RunRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register `run_id` and return the receiver its dispatch loop watches
    /// (`true` = cancel requested) plus the guard that unregisters it.
    /// Re-registering an id replaces the previous sender.
    pub fn register(&self, run_id: &str) -> (watch::Receiver<bool>, RunRegistration) {
        let (tx, rx) = watch::channel(false);
        self.inner.lock().insert(run_id.to_string(), tx);
        let registration = RunRegistration {
            registry: self.clone(),
            run_id: run_id.to_string(),
        };
        (rx, registration)
    }

    /// True when `run_id` is owned by this process.
    pub fn is_live(&self, run_id: &str) -> bool {
        self.inner.lock().contains_key(run_id)
    }

    /// Signal cancellation to an in-process run. Returns false when the run is
    /// not registered here. Callers commit Canceling to SQLite first.
    pub fn signal_cancel(&self, run_id: &str) -> bool {
        match self.inner.lock().get(run_id) {
            Some(tx) => {
                tx.send_replace(true);
                true
            }
            None => false,
        }
    }

    /// Registered run ids (unordered).
    pub fn live_run_ids(&self) -> Vec<String> {
        self.inner.lock().keys().cloned().collect()
    }

    fn unregister(&self, run_id: &str) {
        self.inner.lock().remove(run_id);
    }
}

/// Unregisters its run from the [`RunRegistry`] on drop.
pub struct RunRegistration {
    registry: RunRegistry,
    run_id: String,
}

impl RunRegistration {
    pub fn run_id(&self) -> &str {
        &self.run_id
    }
}

impl Drop for RunRegistration {
    fn drop(&mut self) {
        self.registry.unregister(&self.run_id);
    }
}

impl std::fmt::Debug for RunRegistration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunRegistration")
            .field("run_id", &self.run_id)
            .finish()
    }
}
