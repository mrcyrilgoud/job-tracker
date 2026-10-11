//! Pure run-status transition function and terminal-status derivation.
//!
//! Encodes the run state diagram from design.md exactly:
//!
//! ```text
//! Queued    --FirstUnitStarted-->            Active
//! Queued    --CancelAccepted-->              Canceling
//! Active    --CancelAccepted-->              Canceling
//! Active    --AllUnitsSettled{any_error}-->  CompletedWithErrors
//! Active    --AllUnitsSettled{!any_error}--> Completed
//! Canceling --AllUnitsSettled-->             Canceled
//! ```
//!
//! Every other (status, event) pair, including any event on a terminal
//! status, is rejected with [`LifecycleError`].

use super::model::RunStatus;

/// An event that may move a run to a new [`RunStatus`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RunEvent {
    /// The first unit of work started (Req 1.3).
    FirstUnitStarted,
    /// A Cancellation_Request was accepted (Req 5.3).
    CancelAccepted,
    /// Every scheduled unit reached a terminal status. `any_error` is true
    /// when at least one unit (posting or stage) ended in Error (Req 1.5, 1.6, 5.6).
    AllUnitsSettled { any_error: bool },
}

impl RunEvent {
    /// Stable name for diagnostics and error messages.
    pub fn name(self) -> &'static str {
        match self {
            Self::FirstUnitStarted => "first_unit_started",
            Self::CancelAccepted => "cancel_accepted",
            Self::AllUnitsSettled { .. } => "all_units_settled",
        }
    }
}

/// Rejected lifecycle transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LifecycleError {
    /// The current status is terminal; terminal statuses accept no events (Req 1.4).
    #[error("run is terminal ({}); rejected event {}", from.as_str(), event.name())]
    Terminal { from: RunStatus, event: RunEvent },
    /// The event is not a legal transition from the current non-terminal status.
    #[error("illegal run transition from {} on {}", from.as_str(), event.name())]
    IllegalTransition { from: RunStatus, event: RunEvent },
}

/// Pure transition function. Returns the successor defined by the run state
/// diagram, or an error for any transition not in the diagram.
pub fn next_status(current: RunStatus, event: RunEvent) -> Result<RunStatus, LifecycleError> {
    use RunEvent as E;
    use RunStatus as S;

    if current.is_terminal() {
        return Err(LifecycleError::Terminal {
            from: current,
            event,
        });
    }

    match (current, event) {
        (S::Queued, E::FirstUnitStarted) => Ok(S::Active),
        (S::Queued | S::Active, E::CancelAccepted) => Ok(S::Canceling),
        (S::Active, E::AllUnitsSettled { any_error: true }) => Ok(S::CompletedWithErrors),
        (S::Active, E::AllUnitsSettled { any_error: false }) => Ok(S::Completed),
        (S::Canceling, E::AllUnitsSettled { .. }) => Ok(S::Canceled),
        _ => Err(LifecycleError::IllegalTransition {
            from: current,
            event,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use RunEvent as E;
    use RunStatus as S;

    const ALL_EVENTS: [RunEvent; 4] = [
        E::FirstUnitStarted,
        E::CancelAccepted,
        E::AllUnitsSettled { any_error: false },
        E::AllUnitsSettled { any_error: true },
    ];

    /// The full legal-transition table from the design state diagram.
    fn expected(current: RunStatus, event: RunEvent) -> Option<RunStatus> {
        match (current, event) {
            (S::Queued, E::FirstUnitStarted) => Some(S::Active),
            (S::Queued, E::CancelAccepted) => Some(S::Canceling),
            (S::Active, E::CancelAccepted) => Some(S::Canceling),
            (S::Active, E::AllUnitsSettled { any_error: false }) => Some(S::Completed),
            (S::Active, E::AllUnitsSettled { any_error: true }) => Some(S::CompletedWithErrors),
            (S::Canceling, E::AllUnitsSettled { .. }) => Some(S::Canceled),
            _ => None,
        }
    }

    #[test]
    fn matches_state_diagram_for_every_pair() {
        for status in S::ALL {
            for event in ALL_EVENTS {
                let got = next_status(status, event);
                match expected(status, event) {
                    Some(to) => assert_eq!(got, Ok(to), "{status:?} + {event:?}"),
                    None => assert!(got.is_err(), "{status:?} + {event:?} should be rejected"),
                }
            }
        }
    }

    #[test]
    fn first_unit_moves_queued_to_active() {
        assert_eq!(next_status(S::Queued, E::FirstUnitStarted), Ok(S::Active));
        assert!(next_status(S::Active, E::FirstUnitStarted).is_err());
        assert!(next_status(S::Canceling, E::FirstUnitStarted).is_err());
    }

    #[test]
    fn settled_resolves_by_error_flag_and_cancel_state() {
        assert_eq!(
            next_status(S::Active, E::AllUnitsSettled { any_error: false }),
            Ok(S::Completed)
        );
        assert_eq!(
            next_status(S::Active, E::AllUnitsSettled { any_error: true }),
            Ok(S::CompletedWithErrors)
        );
        // Cancellation wins regardless of unit errors (Req 5.6).
        for any_error in [false, true] {
            assert_eq!(
                next_status(S::Canceling, E::AllUnitsSettled { any_error }),
                Ok(S::Canceled)
            );
        }
        // Not in the diagram: settling straight from Queued.
        assert!(matches!(
            next_status(S::Queued, E::AllUnitsSettled { any_error: false }),
            Err(LifecycleError::IllegalTransition {
                from: S::Queued,
                ..
            })
        ));
    }

    #[test]
    fn cancel_only_from_queued_or_active() {
        assert_eq!(next_status(S::Queued, E::CancelAccepted), Ok(S::Canceling));
        assert_eq!(next_status(S::Active, E::CancelAccepted), Ok(S::Canceling));
        assert!(matches!(
            next_status(S::Canceling, E::CancelAccepted),
            Err(LifecycleError::IllegalTransition { .. })
        ));
    }

    #[test]
    fn terminal_statuses_reject_every_event() {
        for status in S::ALL.into_iter().filter(|s| s.is_terminal()) {
            for event in ALL_EVENTS {
                assert_eq!(
                    next_status(status, event),
                    Err(LifecycleError::Terminal {
                        from: status,
                        event
                    }),
                );
            }
        }
    }

    #[test]
    fn error_messages_name_status_and_event() {
        let e = next_status(S::Completed, E::CancelAccepted).unwrap_err();
        assert_eq!(
            e.to_string(),
            "run is terminal (completed); rejected event cancel_accepted"
        );
        let e = next_status(S::Canceling, E::FirstUnitStarted).unwrap_err();
        assert_eq!(
            e.to_string(),
            "illegal run transition from canceling on first_unit_started"
        );
    }
}
