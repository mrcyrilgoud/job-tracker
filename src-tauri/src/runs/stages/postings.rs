//! Posting-check stage: DB-free workers that evaluate postings under the 30 s timeout.
//!
//! Design.md, "Worker" (Component 5). The coordinator's dispatch loop owns the
//! SQLite connection, the ledger, and the event sink. A worker only does
//! network I/O through the injected [`PostingFetcher`] and reports exactly one
//! [`WorkerMsg`] per dispatched posting over an unbounded `mpsc` channel,
//! attributed by `job_id` (Req 4.6, 4.7):
//!
//! - The evaluation runs in its own task, so a panic surfaces as a
//!   `JoinError`, which maps to [`FailureCategory::Internal`].
//! - After `eval_timeout` (30 s) the worker reports authoritative
//!   [`CheckEvidence::timeout`] evidence, which classifies as Unknown with
//!   category `timeout` (Req 8.1, 8.2). It then keeps the network task for up
//!   to `late_grace` (15 s), holding its permit, so that a slow evaluation can
//!   still yield *supplementary* evidence (Req 8.8, 8.9). The message sequence
//!   in that case is `Authoritative(timeout)` → `LateStarted` → then either
//!   `Supplementary { evidence }` (the late task finished in time) or
//!   `LateAbandoned` (the grace deadline passed or the task was aborted). The
//!   supplementary result never changes the posting row, its state, or the
//!   counters; it is audit-only.
//! - The worker holds its semaphore permit until it returns (including across
//!   the late-grace window), so the number of running evaluations never
//!   exceeds the coordinator's permit count (Req 4.1).
//! - If the worker is dropped or aborted before reporting (for example the run
//!   failed and the coordinator dropped its `JoinSet`, or the worker panicked),
//!   its [`Reporter`] sends a `Failed { Internal }` message from `Drop`, so the
//!   loop can never wait forever on a posting that has no worker. The inner
//!   evaluation task is aborted on drop as well.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, OwnedSemaphorePermit};
use tokio::task::JoinHandle;

use crate::jobs::posting_check::evidence::{CheckEvidence, FailureCategory};
use crate::jobs::posting_check::fetch::PostingFetcher;
use crate::jobs::posting_check::provider::ProviderListingCache;
use crate::jobs::posting_check::{evaluate, PostingCheckInput};

/// Reason recorded when the evaluation task panicked.
pub const REASON_PANICKED: &str = "The posting check stopped unexpectedly (internal error)";
/// Reason recorded when the worker stopped before reporting a result.
pub const REASON_ABANDONED: &str = "The posting check stopped before reporting a result";

/// One worker's report to the dispatch loop.
///
/// A worker sends exactly one *status-affecting* message per dispatched
/// posting — either [`WorkerMsg::Authoritative`] or [`WorkerMsg::Failed`] —
/// which the loop counts as the posting's completion. When the authoritative
/// message is a timeout, the worker additionally emits a [`WorkerMsg::LateStarted`]
/// marker and then, once the late-grace window resolves, exactly one of
/// [`WorkerMsg::Supplementary`] or [`WorkerMsg::LateAbandoned`]. Those late
/// messages track their own accounting (`late_in_flight`) and never touch the
/// posting's status or the counters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerMsg {
    /// Authoritative evidence for the attempt: the real evaluation, or the
    /// timeout evidence when the 30 s bound elapsed.
    Authoritative {
        job_id: String,
        evidence: CheckEvidence,
    },
    /// No evidence could be produced (panic, abort). The posting becomes Error.
    Failed {
        job_id: String,
        category: FailureCategory,
        reason: String,
    },
    /// The 30 s bound elapsed and a late-grace window opened. The worker still
    /// holds its permit; a `Supplementary` or `LateAbandoned` will follow.
    LateStarted { job_id: String },
    /// The network task finished within the grace window after a timeout. Its
    /// evidence is stored as supplementary (audit-only); the posting row,
    /// state, and counters are unchanged (Req 8.8, 8.9).
    Supplementary {
        job_id: String,
        evidence: CheckEvidence,
    },
    /// The grace window closed (deadline passed, cancel, panic, or abort)
    /// without a late result. No supplementary evidence is stored.
    LateAbandoned { job_id: String },
}

impl WorkerMsg {
    pub fn job_id(&self) -> &str {
        match self {
            Self::Authoritative { job_id, .. }
            | Self::Failed { job_id, .. }
            | Self::LateStarted { job_id }
            | Self::Supplementary { job_id, .. }
            | Self::LateAbandoned { job_id } => job_id,
        }
    }
}

/// Everything a worker needs for one posting. Built by the coordinator at
/// dispatch time from the accepted run's inputs; no DB handle.
pub struct WorkerTask<F> {
    pub input: PostingCheckInput,
    pub fetcher: Arc<F>,
    /// Per-run listing memo shared by every worker of the run.
    pub cache: ProviderListingCache,
    /// The dispatch time, also written to `run_postings.attempted_at`.
    pub attempted_at: String,
    pub eval_timeout: Duration,
    /// How long a timed-out network task may keep running for supplementary
    /// evidence, holding its permit (Req 8.8).
    pub late_grace: Duration,
}

/// Sends the worker's single message; sends `Failed { Internal }` from `Drop`
/// if nothing was sent (worker aborted or panicked).
struct Reporter {
    job_id: String,
    tx: mpsc::UnboundedSender<WorkerMsg>,
    sent: bool,
}

impl Reporter {
    fn send(mut self, msg: WorkerMsg) {
        self.sent = true;
        // The loop may already be gone (run failed); nothing to report to then.
        let _ = self.tx.send(msg);
    }
}

impl Drop for Reporter {
    fn drop(&mut self) {
        if !self.sent {
            let _ = self.tx.send(WorkerMsg::Failed {
                job_id: std::mem::take(&mut self.job_id),
                category: FailureCategory::Internal,
                reason: REASON_ABANDONED.to_string(),
            });
        }
    }
}

/// Aborts the evaluation task when dropped, so an aborted worker does not
/// leave a detached network task behind.
struct AbortOnDrop<T>(JoinHandle<T>);

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Evaluate one posting and report the result. Spawned by the coordinator;
/// never touches the database.
pub async fn run_worker<F: PostingFetcher>(
    task: WorkerTask<F>,
    permit: OwnedSemaphorePermit,
    tx: mpsc::UnboundedSender<WorkerMsg>,
) {
    let WorkerTask {
        input,
        fetcher,
        cache,
        attempted_at,
        eval_timeout,
        late_grace,
    } = task;
    let job_id = input.identity.job_id.clone();
    let reporter = Reporter {
        job_id: job_id.clone(),
        tx: tx.clone(),
        sent: false,
    };
    let identity = input.identity.clone();

    let eval_at = attempted_at.clone();
    let mut handle = AbortOnDrop(tokio::spawn(async move {
        evaluate(&input, fetcher.as_ref(), &cache, eval_at).await
    }));

    match tokio::time::timeout(eval_timeout, &mut handle.0).await {
        Ok(Ok(evidence)) => {
            reporter.send(WorkerMsg::Authoritative { job_id, evidence });
        }
        Ok(Err(join_err)) => {
            reporter.send(WorkerMsg::Failed {
                job_id,
                category: FailureCategory::Internal,
                reason: if join_err.is_panic() {
                    REASON_PANICKED
                } else {
                    REASON_ABANDONED
                }
                .to_string(),
            });
        }
        // Req 8.2: finalize at the bound as Completed/Unknown(timeout). Then
        // keep the network task for the late-grace window (Req 8.8, 8.9): its
        // result, if any, is supplementary and never changes the posting.
        Err(_elapsed) => {
            // Open the `late_in_flight` slot BEFORE the authoritative timeout,
            // so the loop increments `late_in_flight` before it decrements
            // `in_flight` for this posting. Otherwise the loop could observe
            // `in_flight == 0 && late_in_flight == 0` and break between the two
            // messages, dropping the late result.
            let _ = tx.send(WorkerMsg::LateStarted {
                job_id: job_id.clone(),
            });
            reporter.send(WorkerMsg::Authoritative {
                job_id: job_id.clone(),
                evidence: CheckEvidence::timeout(&identity, attempted_at),
            });
            let late = match tokio::time::timeout(late_grace, &mut handle.0).await {
                Ok(Ok(evidence)) => WorkerMsg::Supplementary { job_id, evidence },
                // Grace deadline passed, or the task panicked/was aborted: no
                // supplementary evidence, but the loop must close the slot.
                _ => WorkerMsg::LateAbandoned { job_id },
            };
            let _ = tx.send(late);
        }
    }
    drop(handle);
    // Released last: the permit covers the whole evaluation, including the
    // late-grace window (Req 4.1, 8.8).
    drop(permit);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::posting_check::evidence::Provider;
    use crate::jobs::posting_check::fetch::PageFetch;
    use crate::jobs::posting_check::provider::ListingFetch;
    use crate::runs::model::JobIdentity;
    use tokio::sync::Semaphore;

    struct SlowFetcher {
        delay: Duration,
        panic: bool,
    }

    impl PostingFetcher for SlowFetcher {
        async fn fetch_page(&self, url: &str) -> PageFetch {
            tokio::time::sleep(self.delay).await;
            if self.panic {
                panic!("scripted fetch panic");
            }
            PageFetch {
                requested_url: url.into(),
                final_url: url.into(),
                http_status: Some(404),
                redirect_statuses: vec![],
                signal_headers: vec![],
                body: String::new(),
                error_kind: None,
            }
        }

        async fn fetch_listing(&self, _: Provider, _: &str) -> ListingFetch {
            unreachable!("no provider target in these tests")
        }
    }

    fn task(delay: Duration, panic: bool) -> WorkerTask<SlowFetcher> {
        task_with_grace(delay, panic, Duration::from_secs(15))
    }

    fn task_with_grace(
        delay: Duration,
        panic: bool,
        late_grace: Duration,
    ) -> WorkerTask<SlowFetcher> {
        WorkerTask {
            input: PostingCheckInput::new(JobIdentity {
                job_id: "j1".into(),
                title: "Engineer".into(),
                company_name: "Acme".into(),
                posting_url: "https://acme.example/jobs/1".into(),
            }),
            fetcher: Arc::new(SlowFetcher { delay, panic }),
            cache: ProviderListingCache::new(),
            attempted_at: "2026-03-01T10:00:00.000Z".into(),
            eval_timeout: Duration::from_secs(30),
            late_grace,
        }
    }

    async fn run_one(delay: Duration, panic: bool) -> (WorkerMsg, usize) {
        // A short grace so a slow-but-not-timing-out delay produces exactly one
        // message; the timeout-specific tests use the multi-message helper.
        let sem = Arc::new(Semaphore::new(1));
        let permit = sem.clone().acquire_owned().await.unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();
        run_worker(task(delay, panic), permit, tx).await;
        let msg = rx.recv().await.unwrap();
        assert!(rx.try_recv().is_err(), "exactly one message per posting");
        (msg, sem.available_permits())
    }

    /// Drain every message a worker sends and the permits available afterward.
    async fn run_all(t: WorkerTask<SlowFetcher>) -> (Vec<WorkerMsg>, usize) {
        let sem = Arc::new(Semaphore::new(1));
        let permit = sem.clone().acquire_owned().await.unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();
        run_worker(t, permit, tx).await;
        let mut msgs = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            msgs.push(msg);
        }
        (msgs, sem.available_permits())
    }

    #[tokio::test(start_paused = true)]
    async fn reports_authoritative_evidence_and_releases_the_permit() {
        let (msg, permits) = run_one(Duration::from_secs(1), false).await;
        match msg {
            WorkerMsg::Authoritative { job_id, evidence } => {
                assert_eq!(job_id, "j1");
                assert_eq!(evidence.http_status, Some(404));
                assert_eq!(evidence.attempted_at, "2026-03-01T10:00:00.000Z");
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(permits, 1);
    }

    #[tokio::test(start_paused = true)]
    async fn timeout_then_late_finish_reports_authoritative_late_started_and_supplementary() {
        // Eval finishes at 40 s: after the 30 s bound but inside a 15 s grace.
        let (msgs, permits) = run_all(task(Duration::from_secs(40), false)).await;
        assert_eq!(
            msgs.len(),
            3,
            "late_started + authoritative + supplementary: {msgs:?}"
        );
        // The late slot is opened before the authoritative timeout so the loop
        // never observes an empty in-flight count between the two.
        assert_eq!(
            msgs[0],
            WorkerMsg::LateStarted {
                job_id: "j1".into()
            }
        );
        match &msgs[1] {
            WorkerMsg::Authoritative { evidence, .. } => {
                assert_eq!(evidence.failure, Some(FailureCategory::Timeout));
            }
            other => panic!("unexpected second message {other:?}"),
        }
        match &msgs[2] {
            WorkerMsg::Supplementary { job_id, evidence } => {
                assert_eq!(job_id, "j1");
                // The late fetch returned a real 404 page, not the timeout stub.
                assert_eq!(evidence.http_status, Some(404));
            }
            other => panic!("unexpected third message {other:?}"),
        }
        // Permit held across the whole grace window and released at the end.
        assert_eq!(permits, 1);
    }

    #[tokio::test(start_paused = true)]
    async fn timeout_with_no_late_finish_abandons_without_supplementary() {
        // Eval never finishes within the 30 s bound + 15 s grace.
        let (msgs, permits) = run_all(task(Duration::from_secs(600), false)).await;
        assert_eq!(
            msgs.len(),
            3,
            "late_started + authoritative + late_abandoned: {msgs:?}"
        );
        assert_eq!(
            msgs[0],
            WorkerMsg::LateStarted {
                job_id: "j1".into()
            }
        );
        assert!(matches!(msgs[1], WorkerMsg::Authoritative { .. }));
        assert_eq!(
            msgs[2],
            WorkerMsg::LateAbandoned {
                job_id: "j1".into()
            }
        );
        assert_eq!(permits, 1);
    }

    #[tokio::test(start_paused = true)]
    async fn timeout_with_late_panic_abandons_without_supplementary() {
        // Eval would resolve inside the grace window but panics instead.
        let (msgs, _) = run_all(task_with_grace(
            Duration::from_secs(40),
            true,
            Duration::from_secs(15),
        ))
        .await;
        assert_eq!(msgs.len(), 3, "{msgs:?}");
        assert_eq!(
            msgs[0],
            WorkerMsg::LateStarted {
                job_id: "j1".into()
            }
        );
        assert!(matches!(msgs[1], WorkerMsg::Authoritative { .. }));
        assert_eq!(
            msgs[2],
            WorkerMsg::LateAbandoned {
                job_id: "j1".into()
            }
        );
    }

    #[tokio::test(start_paused = true)]
    async fn panic_maps_to_internal() {
        let (msg, _) = run_one(Duration::from_millis(5), true).await;
        assert_eq!(
            msg,
            WorkerMsg::Failed {
                job_id: "j1".into(),
                category: FailureCategory::Internal,
                reason: REASON_PANICKED.into(),
            }
        );
    }

    #[tokio::test(start_paused = true)]
    async fn aborted_worker_reports_failed_from_drop() {
        let sem = Arc::new(Semaphore::new(1));
        let permit = sem.clone().acquire_owned().await.unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let handle = tokio::spawn(run_worker(task(Duration::from_secs(10), false), permit, tx));
        tokio::task::yield_now().await;
        handle.abort();
        let _ = handle.await;
        match rx.recv().await.unwrap() {
            WorkerMsg::Failed {
                job_id, category, ..
            } => {
                assert_eq!(
                    (job_id.as_str(), category),
                    ("j1", FailureCategory::Internal)
                );
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(sem.available_permits(), 1);
    }
}
