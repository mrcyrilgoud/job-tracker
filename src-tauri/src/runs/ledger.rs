//! Pure in-memory `RunLedger`: per-posting status and counters for one run.
//!
//! The ledger mirrors the `run_postings` rows of a single run. The coordinator
//! applies each committed per-posting transition here to build event deltas,
//! and property tests use it as the reference model.
//!
//! Invariants (Req 3.5, 3.10, 3.11, 8.3, 8.4, 8.6):
//! - Every job id appears exactly once, with one stable `JobIdentity`.
//! - Only `Queued→Active|Canceled|Error` and `Active→Completed|Error` are legal.
//!   Terminal entries are immutable, so counters move exactly once per terminal
//!   transition and an Error is never followed by Completed.
//! - `queued + active + completed + error + canceled == total` after every call.
//! - A rejected request leaves the ledger unchanged.

use std::collections::HashMap;

use thiserror::Error;

use super::model::{JobIdentity, PostingCounts, PostingState, PostingStatus};

/// One posting's position in a run. Task 2.1 maps this to the wire `PostingProgress`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostingEntry {
    /// Stable 0-based display order within the run (`run_postings.ordinal`).
    pub ordinal: usize,
    /// Frozen Job_Identity for the lifetime of the run.
    pub identity: JobIdentity,
    pub status: PostingStatus,
    /// Present only when `status == Completed`.
    pub posting_state: Option<PostingState>,
    pub reason_code: Option<String>,
    /// Classification_Reason (Completed) or failure reason (Error).
    pub reason: Option<String>,
    pub failure_category: Option<String>,
    pub attempted_at: Option<String>,
    pub finished_at: Option<String>,
}

impl PostingEntry {
    fn queued(ordinal: usize, identity: JobIdentity) -> Self {
        Self {
            ordinal,
            identity,
            status: PostingStatus::Queued,
            posting_state: None,
            reason_code: None,
            reason: None,
            failure_category: None,
            attempted_at: None,
            finished_at: None,
        }
    }

    pub fn job_id(&self) -> &str {
        &self.identity.job_id
    }

    pub fn is_retry_eligible(&self) -> bool {
        is_retry_eligible(self.status, self.posting_state)
    }

    pub fn needs_attention(&self) -> bool {
        needs_attention(self.status, self.posting_state)
    }
}

/// Detail recorded with a transition. `Some` fields overwrite the entry; `None`
/// fields keep what an earlier transition recorded (for example `attempted_at`
/// set at Active survives the Completed transition).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TransitionDetail {
    pub posting_state: Option<PostingState>,
    pub reason_code: Option<String>,
    pub reason: Option<String>,
    pub failure_category: Option<String>,
    pub attempted_at: Option<String>,
    pub finished_at: Option<String>,
}

impl TransitionDetail {
    /// Queued → Active.
    pub fn started(attempted_at: impl Into<String>) -> Self {
        Self {
            attempted_at: Some(attempted_at.into()),
            ..Self::default()
        }
    }

    /// Active → Completed with a Posting_State and Classification_Reason.
    pub fn completed(
        state: PostingState,
        reason_code: impl Into<String>,
        reason: impl Into<String>,
        finished_at: impl Into<String>,
    ) -> Self {
        Self {
            posting_state: Some(state),
            reason_code: Some(reason_code.into()),
            reason: Some(reason.into()),
            finished_at: Some(finished_at.into()),
            ..Self::default()
        }
    }

    /// → Error with a failure category and non-empty reason.
    pub fn error(
        failure_category: impl Into<String>,
        reason: impl Into<String>,
        finished_at: impl Into<String>,
    ) -> Self {
        Self {
            failure_category: Some(failure_category.into()),
            reason: Some(reason.into()),
            finished_at: Some(finished_at.into()),
            ..Self::default()
        }
    }

    /// Queued → Canceled.
    pub fn canceled(finished_at: impl Into<String>) -> Self {
        Self {
            finished_at: Some(finished_at.into()),
            ..Self::default()
        }
    }
}

/// Result of an applied transition: the prior status and the updated entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostingDelta {
    pub previous: PostingStatus,
    pub entry: PostingEntry,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LedgerError {
    #[error("duplicate job id in run: {0}")]
    DuplicateJobId(String),
    #[error("job id not in run: {0}")]
    UnknownJob(String),
    #[error("illegal posting transition for {job_id}: {} -> {}", from.as_str(), to.as_str())]
    IllegalTransition {
        job_id: String,
        from: PostingStatus,
        to: PostingStatus,
    },
    #[error("invalid transition detail for {job_id}: {message}")]
    InvalidDetail {
        job_id: String,
        message: &'static str,
    },
}

/// True for the transitions in the per-posting state diagram.
pub fn is_legal_transition(from: PostingStatus, to: PostingStatus) -> bool {
    use PostingStatus::*;
    matches!(
        (from, to),
        (Queued, Active)
            | (Queued, Canceled)
            | (Queued, Error)
            | (Active, Completed)
            | (Active, Error)
    )
}

/// Retry eligibility (Req 5.8): Posting_State Unknown, or status Error or Canceled.
pub fn is_retry_eligible(status: PostingStatus, state: Option<PostingState>) -> bool {
    state == Some(PostingState::Unknown)
        || matches!(status, PostingStatus::Error | PostingStatus::Canceled)
}

/// "Needs attention" filter (Req 9.7): Posting_State Unknown, or status Error.
pub fn needs_attention(status: PostingStatus, state: Option<PostingState>) -> bool {
    state == Some(PostingState::Unknown) || status == PostingStatus::Error
}

fn counter_mut(counts: &mut PostingCounts, status: PostingStatus) -> &mut u64 {
    match status {
        PostingStatus::Queued => &mut counts.queued,
        PostingStatus::Active => &mut counts.active,
        PostingStatus::Completed => &mut counts.completed,
        PostingStatus::Error => &mut counts.error,
        PostingStatus::Canceled => &mut counts.canceled,
    }
}

fn is_blank(s: &Option<String>) -> bool {
    s.as_deref().map_or(true, |r| r.trim().is_empty())
}

#[derive(Debug, Clone)]
pub struct RunLedger {
    /// Stable ordinal order.
    entries: Vec<PostingEntry>,
    /// job_id → index into `entries`.
    index: HashMap<String, usize>,
    counts: PostingCounts,
    /// Every entry before this index has left Queued. Entries never return to
    /// Queued, so the cursor only moves forward.
    queued_cursor: usize,
}

impl RunLedger {
    /// Build a ledger with every identity Queued, in the given order.
    /// Rejects duplicate job ids.
    pub fn new(identities: Vec<JobIdentity>) -> Result<Self, LedgerError> {
        let mut index = HashMap::with_capacity(identities.len());
        let mut entries = Vec::with_capacity(identities.len());
        for (ordinal, identity) in identities.into_iter().enumerate() {
            if index.insert(identity.job_id.clone(), ordinal).is_some() {
                return Err(LedgerError::DuplicateJobId(identity.job_id));
            }
            entries.push(PostingEntry::queued(ordinal, identity));
        }
        let counts = PostingCounts {
            queued: entries.len() as u64,
            ..PostingCounts::default()
        };
        Ok(Self {
            entries,
            index,
            counts,
            queued_cursor: 0,
        })
    }

    /// Apply one posting transition. On any error the ledger is unchanged.
    pub fn transition(
        &mut self,
        job_id: &str,
        to: PostingStatus,
        detail: TransitionDetail,
    ) -> Result<PostingDelta, LedgerError> {
        let &idx = self
            .index
            .get(job_id)
            .ok_or_else(|| LedgerError::UnknownJob(job_id.to_string()))?;
        let from = self.entries[idx].status;
        if !is_legal_transition(from, to) {
            return Err(LedgerError::IllegalTransition {
                job_id: job_id.to_string(),
                from,
                to,
            });
        }
        Self::validate_detail(job_id, to, &detail)?;

        let entry = &mut self.entries[idx];
        entry.status = to;
        entry.posting_state = detail.posting_state;
        if detail.reason_code.is_some() {
            entry.reason_code = detail.reason_code;
        }
        if detail.reason.is_some() {
            entry.reason = detail.reason;
        }
        if detail.failure_category.is_some() {
            entry.failure_category = detail.failure_category;
        }
        if detail.attempted_at.is_some() {
            entry.attempted_at = detail.attempted_at;
        }
        if detail.finished_at.is_some() {
            entry.finished_at = detail.finished_at;
        }
        let entry = entry.clone();

        *counter_mut(&mut self.counts, from) -= 1;
        *counter_mut(&mut self.counts, to) += 1;
        if from == PostingStatus::Queued {
            self.advance_queued_cursor();
        }
        debug_assert_eq!(self.counts.total(), self.entries.len() as u64);

        Ok(PostingDelta {
            previous: from,
            entry,
        })
    }

    /// Completed carries a Posting_State and non-empty reason (Req 3.8);
    /// Error carries a non-empty reason (Req 3.4); nothing else carries a state.
    fn validate_detail(
        job_id: &str,
        to: PostingStatus,
        detail: &TransitionDetail,
    ) -> Result<(), LedgerError> {
        let invalid = |message| {
            Err(LedgerError::InvalidDetail {
                job_id: job_id.to_string(),
                message,
            })
        };
        match to {
            PostingStatus::Completed => {
                if detail.posting_state.is_none() {
                    return invalid("completed requires a posting state");
                }
                if is_blank(&detail.reason) {
                    return invalid("completed requires a non-empty reason");
                }
            }
            PostingStatus::Error => {
                if detail.posting_state.is_some() {
                    return invalid("only completed postings carry a posting state");
                }
                if is_blank(&detail.reason) {
                    return invalid("error requires a non-empty reason");
                }
            }
            _ => {
                if detail.posting_state.is_some() {
                    return invalid("only completed postings carry a posting state");
                }
            }
        }
        Ok(())
    }

    fn advance_queued_cursor(&mut self) {
        while self
            .entries
            .get(self.queued_cursor)
            .is_some_and(|e| e.status != PostingStatus::Queued)
        {
            self.queued_cursor += 1;
        }
    }

    /// Counters by Posting_Status. Their sum always equals `total()`.
    pub fn counts(&self) -> PostingCounts {
        self.counts
    }

    pub fn total(&self) -> u64 {
        self.entries.len() as u64
    }

    pub fn has_queued(&self) -> bool {
        self.counts.queued > 0
    }

    /// True when no posting is Queued or Active.
    pub fn is_settled(&self) -> bool {
        self.counts.queued == 0 && self.counts.active == 0
    }

    /// Job ids still Queued, in ordinal order.
    pub fn queued_ids(&self) -> impl Iterator<Item = &str> {
        self.entries[self.queued_cursor.min(self.entries.len())..]
            .iter()
            .filter(|e| e.status == PostingStatus::Queued)
            .map(|e| e.job_id())
    }

    /// The lowest-ordinal Queued posting, if any.
    pub fn next_queued(&self) -> Option<&JobIdentity> {
        self.entries[self.queued_cursor.min(self.entries.len())..]
            .iter()
            .find(|e| e.status == PostingStatus::Queued)
            .map(|e| &e.identity)
    }

    pub fn get(&self, job_id: &str) -> Option<&PostingEntry> {
        self.index.get(job_id).map(|&i| &self.entries[i])
    }

    pub fn entries(&self) -> &[PostingEntry] {
        &self.entries
    }

    /// Full list of entries in ordinal order.
    pub fn snapshot(&self) -> Vec<PostingEntry> {
        self.entries.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use PostingStatus::*;

    fn ident(id: &str) -> JobIdentity {
        JobIdentity {
            job_id: id.into(),
            title: format!("Title {id}"),
            company_name: format!("Company {id}"),
            posting_url: format!("https://example.com/jobs/{id}"),
        }
    }

    fn ledger(ids: &[&str]) -> RunLedger {
        RunLedger::new(ids.iter().map(|id| ident(id)).collect()).unwrap()
    }

    fn done(state: PostingState) -> TransitionDetail {
        TransitionDetail::completed(state, "listed_open", "Open: listed", "t2")
    }

    fn assert_sum(l: &RunLedger) {
        assert_eq!(l.counts().total(), l.total());
    }

    #[test]
    fn new_queues_every_identity_in_order() {
        let l = ledger(&["a", "b", "c"]);
        assert_eq!(
            l.counts(),
            PostingCounts {
                queued: 3,
                ..Default::default()
            }
        );
        assert_eq!(l.total(), 3);
        assert!(l.has_queued());
        assert!(!l.is_settled());
        assert_eq!(l.queued_ids().collect::<Vec<_>>(), ["a", "b", "c"]);
        assert_eq!(l.next_queued().unwrap().job_id, "a");
        let snap = l.snapshot();
        assert_eq!(
            snap.iter().map(|e| e.ordinal).collect::<Vec<_>>(),
            [0, 1, 2]
        );
        assert!(snap
            .iter()
            .all(|e| e.status == Queued && e.posting_state.is_none()));
        assert_eq!(snap[1].identity, ident("b"));
    }

    #[test]
    fn new_rejects_duplicate_job_ids() {
        let err = RunLedger::new(vec![ident("a"), ident("b"), ident("a")]).unwrap_err();
        assert_eq!(err, LedgerError::DuplicateJobId("a".into()));
    }

    #[test]
    fn empty_ledger_is_settled() {
        let l = ledger(&[]);
        assert_eq!(l.total(), 0);
        assert!(!l.has_queued());
        assert!(l.is_settled());
        assert!(l.next_queued().is_none());
        assert_eq!(l.queued_ids().count(), 0);
    }

    #[test]
    fn happy_path_moves_counters_once_per_transition() {
        let mut l = ledger(&["a", "b"]);
        let d = l
            .transition("a", Active, TransitionDetail::started("t1"))
            .unwrap();
        assert_eq!(d.previous, Queued);
        assert_eq!(d.entry.status, Active);
        assert_eq!(
            l.counts(),
            PostingCounts {
                queued: 1,
                active: 1,
                ..Default::default()
            }
        );
        assert_eq!(l.next_queued().unwrap().job_id, "b");

        let d = l
            .transition("a", Completed, done(PostingState::Active))
            .unwrap();
        assert_eq!(d.previous, Active);
        assert_eq!(d.entry.posting_state, Some(PostingState::Active));
        assert_eq!(d.entry.reason.as_deref(), Some("Open: listed"));
        assert_eq!(
            d.entry.attempted_at.as_deref(),
            Some("t1"),
            "attempted_at kept from Active"
        );
        assert_eq!(d.entry.finished_at.as_deref(), Some("t2"));
        assert_eq!(
            l.counts(),
            PostingCounts {
                queued: 1,
                completed: 1,
                ..Default::default()
            }
        );

        l.transition("b", Canceled, TransitionDetail::canceled("t3"))
            .unwrap();
        assert_eq!(
            l.counts(),
            PostingCounts {
                completed: 1,
                canceled: 1,
                ..Default::default()
            }
        );
        assert!(l.is_settled());
        assert!(!l.has_queued());
        assert_sum(&l);
    }

    #[test]
    fn all_legal_transitions_are_accepted() {
        assert!(is_legal_transition(Queued, Active));
        assert!(is_legal_transition(Queued, Canceled));
        assert!(is_legal_transition(Queued, Error));
        assert!(is_legal_transition(Active, Completed));
        assert!(is_legal_transition(Active, Error));
        let legal = 5;
        let count = PostingStatus::ALL
            .iter()
            .flat_map(|&f| PostingStatus::ALL.iter().map(move |&t| (f, t)))
            .filter(|&(f, t)| is_legal_transition(f, t))
            .count();
        assert_eq!(count, legal);
    }

    #[test]
    fn illegal_transitions_are_rejected_and_leave_ledger_unchanged() {
        let mut l = ledger(&["q", "a", "c", "e", "x"]);
        l.transition("a", Active, TransitionDetail::started("t"))
            .unwrap();
        l.transition("c", Active, TransitionDetail::started("t"))
            .unwrap();
        l.transition("c", Completed, done(PostingState::Inactive))
            .unwrap();
        l.transition("e", Error, TransitionDetail::error("internal", "boom", "t"))
            .unwrap();
        l.transition("x", Canceled, TransitionDetail::canceled("t"))
            .unwrap();

        for (id, from) in [
            ("q", Queued),
            ("a", Active),
            ("c", Completed),
            ("e", Error),
            ("x", Canceled),
        ] {
            for to in PostingStatus::ALL {
                if is_legal_transition(from, to) {
                    continue;
                }
                let before = l.snapshot();
                let counts = l.counts();
                let detail = if to == Completed {
                    done(PostingState::Active)
                } else {
                    TransitionDetail::error("internal", "r", "t")
                };
                let err = l.transition(id, to, detail).unwrap_err();
                assert_eq!(
                    err,
                    LedgerError::IllegalTransition {
                        job_id: id.into(),
                        from,
                        to
                    }
                );
                assert_eq!(l.snapshot(), before);
                assert_eq!(l.counts(), counts);
            }
        }
        assert_sum(&l);
    }

    #[test]
    fn error_is_never_followed_by_completed() {
        let mut l = ledger(&["a"]);
        l.transition("a", Active, TransitionDetail::started("t1"))
            .unwrap();
        l.transition(
            "a",
            Error,
            TransitionDetail::error("persistence", "db locked", "t2"),
        )
        .unwrap();
        let err = l
            .transition("a", Completed, done(PostingState::Active))
            .unwrap_err();
        assert!(matches!(
            err,
            LedgerError::IllegalTransition {
                from: Error,
                to: Completed,
                ..
            }
        ));
        assert_eq!(
            l.counts(),
            PostingCounts {
                error: 1,
                ..Default::default()
            }
        );
        let e = l.get("a").unwrap();
        assert_eq!(e.failure_category.as_deref(), Some("persistence"));
        assert_eq!(e.reason.as_deref(), Some("db locked"));
        assert!(e.posting_state.is_none());
    }

    #[test]
    fn active_to_active_is_rejected() {
        let mut l = ledger(&["a"]);
        l.transition("a", Active, TransitionDetail::started("t1"))
            .unwrap();
        assert!(matches!(
            l.transition("a", Active, TransitionDetail::started("t2")),
            Err(LedgerError::IllegalTransition {
                from: Active,
                to: Active,
                ..
            })
        ));
        assert_eq!(l.get("a").unwrap().attempted_at.as_deref(), Some("t1"));
    }

    #[test]
    fn unknown_job_is_rejected() {
        let mut l = ledger(&["a"]);
        let err = l
            .transition("zzz", Active, TransitionDetail::default())
            .unwrap_err();
        assert_eq!(err, LedgerError::UnknownJob("zzz".into()));
        assert_eq!(l.counts().queued, 1);
    }

    #[test]
    fn invalid_detail_is_rejected_and_leaves_ledger_unchanged() {
        let mut l = ledger(&["a"]);
        l.transition("a", Active, TransitionDetail::started("t1"))
            .unwrap();
        let before = l.snapshot();

        let no_state = TransitionDetail {
            reason: Some("r".into()),
            ..Default::default()
        };
        assert!(matches!(
            l.transition("a", Completed, no_state),
            Err(LedgerError::InvalidDetail { .. })
        ));

        let blank_reason = TransitionDetail::completed(PostingState::Unknown, "c", "  ", "t");
        assert!(matches!(
            l.transition("a", Completed, blank_reason),
            Err(LedgerError::InvalidDetail { .. })
        ));

        let empty_error = TransitionDetail::error("internal", "", "t");
        assert!(matches!(
            l.transition("a", Error, empty_error),
            Err(LedgerError::InvalidDetail { .. })
        ));

        let error_with_state = TransitionDetail {
            posting_state: Some(PostingState::Active),
            ..TransitionDetail::error("internal", "r", "t")
        };
        assert!(matches!(
            l.transition("a", Error, error_with_state),
            Err(LedgerError::InvalidDetail { .. })
        ));

        assert_eq!(l.snapshot(), before);
        assert_eq!(
            l.counts(),
            PostingCounts {
                active: 1,
                ..Default::default()
            }
        );
    }

    #[test]
    fn queued_iteration_skips_out_of_order_starts() {
        let mut l = ledger(&["a", "b", "c", "d"]);
        l.transition("b", Active, TransitionDetail::started("t"))
            .unwrap();
        assert_eq!(l.next_queued().unwrap().job_id, "a");
        assert_eq!(l.queued_ids().collect::<Vec<_>>(), ["a", "c", "d"]);
        l.transition(
            "a",
            Error,
            TransitionDetail::error("job_missing", "gone", "t"),
        )
        .unwrap();
        l.transition("d", Canceled, TransitionDetail::canceled("t"))
            .unwrap();
        assert_eq!(l.next_queued().unwrap().job_id, "c");
        assert_eq!(l.queued_ids().collect::<Vec<_>>(), ["c"]);
        l.transition("c", Active, TransitionDetail::started("t"))
            .unwrap();
        assert!(l.next_queued().is_none());
        assert!(!l.has_queued());
        assert!(!l.is_settled());
        assert_sum(&l);
    }

    #[test]
    fn identity_is_stable_across_transitions() {
        let mut l = ledger(&["a"]);
        l.transition("a", Active, TransitionDetail::started("t1"))
            .unwrap();
        let d = l
            .transition("a", Completed, done(PostingState::Unknown))
            .unwrap();
        assert_eq!(d.entry.identity, ident("a"));
        assert_eq!(d.entry.ordinal, 0);
    }

    #[test]
    fn retry_eligibility_and_attention_predicates() {
        use PostingState as S;
        let cases = [
            (Queued, None, false, false),
            (Active, None, false, false),
            (Completed, Some(S::Active), false, false),
            (Completed, Some(S::Inactive), false, false),
            (Completed, Some(S::Unknown), true, true),
            (Error, None, true, true),
            (Canceled, None, true, false),
        ];
        for (status, state, retry, attention) in cases {
            assert_eq!(
                is_retry_eligible(status, state),
                retry,
                "{status:?}/{state:?}"
            );
            assert_eq!(
                needs_attention(status, state),
                attention,
                "{status:?}/{state:?}"
            );
            // Every row needing attention is retry-eligible.
            assert!(!attention || retry);
        }

        let mut l = ledger(&["u", "e"]);
        l.transition("u", Active, TransitionDetail::started("t"))
            .unwrap();
        l.transition("u", Completed, done(S::Unknown)).unwrap();
        l.transition("e", Error, TransitionDetail::error("internal", "x", "t"))
            .unwrap();
        assert!(l.get("u").unwrap().is_retry_eligible());
        assert!(l.get("u").unwrap().needs_attention());
        assert!(l.get("e").unwrap().needs_attention());
    }
}
