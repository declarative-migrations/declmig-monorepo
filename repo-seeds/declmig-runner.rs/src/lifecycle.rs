use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Pending,
    Running,
    Cancelling,
    Cancelled,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleState {
    pub execution_id: String,
    pub plan_digest: String,
    pub fencing_token: u64,
    pub attempt: u8,
    pub status: ExecutionStatus,
    pub last_event_seq: u64,
    pub last_event_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleEventKind {
    StartAttempt,
    RequestCancel,
    MarkCancelled,
    MarkSucceeded,
    MarkFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleEvent {
    pub execution_id: String,
    pub plan_digest: String,
    pub fencing_token: u64,
    pub event_seq: u64,
    pub event_id: String,
    pub kind: LifecycleEventKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionOutcome {
    Applied,
    Duplicate,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LifecycleError {
    #[error("event execution identity does not match state")]
    IdentityMismatch,
    #[error("event plan digest does not match state")]
    PlanDigestMismatch,
    #[error("event identifier is invalid")]
    InvalidEventId,
    #[error("event sequence regressed or conflicts with committed history")]
    SequenceConflict,
    #[error("event sequence must advance by exactly one")]
    SequenceGap,
    #[error("fencing token is stale or invalid for this transition")]
    InvalidFencingToken,
    #[error("attempt limit reached")]
    AttemptLimitReached,
    #[error("lifecycle transition is invalid")]
    InvalidTransition,
}

pub fn apply_event(
    state: &mut LifecycleState,
    event: &LifecycleEvent,
    max_attempts: u8,
) -> Result<TransitionOutcome, LifecycleError> {
    if state.execution_id != event.execution_id {
        return Err(LifecycleError::IdentityMismatch);
    }
    if state.plan_digest != event.plan_digest {
        return Err(LifecycleError::PlanDigestMismatch);
    }
    if !valid_id(&event.event_id) {
        return Err(LifecycleError::InvalidEventId);
    }

    if event.event_seq == state.last_event_seq {
        return if state.last_event_id.as_deref() == Some(event.event_id.as_str()) {
            Ok(TransitionOutcome::Duplicate)
        } else {
            Err(LifecycleError::SequenceConflict)
        };
    }
    if event.event_seq != state.last_event_seq.saturating_add(1) {
        return Err(LifecycleError::SequenceGap);
    }

    match event.kind {
        LifecycleEventKind::StartAttempt => {
            if !matches!(
                state.status,
                ExecutionStatus::Pending | ExecutionStatus::Cancelled | ExecutionStatus::Failed
            ) {
                return Err(LifecycleError::InvalidTransition);
            }
            if state.attempt >= max_attempts {
                return Err(LifecycleError::AttemptLimitReached);
            }
            let token_is_valid = if state.fencing_token == 0 {
                event.fencing_token > 0
            } else {
                event.fencing_token > state.fencing_token
            };
            if !token_is_valid {
                return Err(LifecycleError::InvalidFencingToken);
            }
            state.fencing_token = event.fencing_token;
            state.attempt += 1;
            state.status = ExecutionStatus::Running;
        }
        LifecycleEventKind::RequestCancel => {
            require_same_fence(state, event)?;
            if state.status != ExecutionStatus::Running {
                return Err(LifecycleError::InvalidTransition);
            }
            state.status = ExecutionStatus::Cancelling;
        }
        LifecycleEventKind::MarkCancelled => {
            require_same_fence(state, event)?;
            if state.status != ExecutionStatus::Cancelling {
                return Err(LifecycleError::InvalidTransition);
            }
            state.status = ExecutionStatus::Cancelled;
        }
        LifecycleEventKind::MarkSucceeded => {
            require_same_fence(state, event)?;
            if state.status != ExecutionStatus::Running {
                return Err(LifecycleError::InvalidTransition);
            }
            state.status = ExecutionStatus::Succeeded;
        }
        LifecycleEventKind::MarkFailed => {
            require_same_fence(state, event)?;
            if !matches!(state.status, ExecutionStatus::Running | ExecutionStatus::Cancelling) {
                return Err(LifecycleError::InvalidTransition);
            }
            state.status = ExecutionStatus::Failed;
        }
    }

    state.last_event_seq = event.event_seq;
    state.last_event_id = Some(event.event_id.clone());
    Ok(TransitionOutcome::Applied)
}

fn require_same_fence(
    state: &LifecycleState,
    event: &LifecycleEvent,
) -> Result<(), LifecycleError> {
    if event.fencing_token == 0 || event.fencing_token != state.fencing_token {
        return Err(LifecycleError::InvalidFencingToken);
    }
    Ok(())
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> LifecycleState {
        LifecycleState {
            execution_id: "exec_1".into(),
            plan_digest: "a".repeat(64),
            fencing_token: 0,
            attempt: 0,
            status: ExecutionStatus::Pending,
            last_event_seq: 0,
            last_event_id: None,
        }
    }

    fn event(seq: u64, token: u64, id: &str, kind: LifecycleEventKind) -> LifecycleEvent {
        LifecycleEvent {
            execution_id: "exec_1".into(),
            plan_digest: "a".repeat(64),
            fencing_token: token,
            event_seq: seq,
            event_id: id.into(),
            kind,
        }
    }

    #[test]
    fn cancellation_is_two_phase_and_replay_safe() {
        let mut state = state();
        let start = event(1, 7, "evt_1", LifecycleEventKind::StartAttempt);
        assert_eq!(
            apply_event(&mut state, &start, 3),
            Ok(TransitionOutcome::Applied)
        );
        assert_eq!(
            apply_event(&mut state, &start, 3),
            Ok(TransitionOutcome::Duplicate)
        );
        assert_eq!(state.status, ExecutionStatus::Running);

        apply_event(
            &mut state,
            &event(2, 7, "evt_2", LifecycleEventKind::RequestCancel),
            3,
        )
        .unwrap();
        assert_eq!(state.status, ExecutionStatus::Cancelling);
        apply_event(
            &mut state,
            &event(3, 7, "evt_3", LifecycleEventKind::MarkCancelled),
            3,
        )
        .unwrap();
        assert_eq!(state.status, ExecutionStatus::Cancelled);
    }

    #[test]
    fn retry_requires_a_strictly_new_fence() {
        let mut state = state();
        apply_event(
            &mut state,
            &event(1, 7, "evt_1", LifecycleEventKind::StartAttempt),
            3,
        )
        .unwrap();
        apply_event(
            &mut state,
            &event(2, 7, "evt_2", LifecycleEventKind::MarkFailed),
            3,
        )
        .unwrap();
        assert_eq!(
            apply_event(
                &mut state,
                &event(3, 7, "evt_3", LifecycleEventKind::StartAttempt),
                3,
            ),
            Err(LifecycleError::InvalidFencingToken)
        );
        apply_event(
            &mut state,
            &event(3, 8, "evt_3", LifecycleEventKind::StartAttempt),
            3,
        )
        .unwrap();
        assert_eq!(state.attempt, 2);
        assert_eq!(state.fencing_token, 8);
    }

    #[test]
    fn stale_fence_sequence_gaps_and_conflicts_fail_closed() {
        let mut state = state();
        apply_event(
            &mut state,
            &event(1, 7, "evt_1", LifecycleEventKind::StartAttempt),
            3,
        )
        .unwrap();
        assert_eq!(
            apply_event(
                &mut state,
                &event(3, 7, "evt_3", LifecycleEventKind::MarkSucceeded),
                3,
            ),
            Err(LifecycleError::SequenceGap)
        );
        assert_eq!(
            apply_event(
                &mut state,
                &event(1, 7, "different", LifecycleEventKind::StartAttempt),
                3,
            ),
            Err(LifecycleError::SequenceConflict)
        );
        assert_eq!(
            apply_event(
                &mut state,
                &event(2, 6, "evt_2", LifecycleEventKind::MarkSucceeded),
                3,
            ),
            Err(LifecycleError::InvalidFencingToken)
        );
    }

    #[test]
    fn terminal_success_cannot_be_cancelled_or_restarted() {
        let mut state = state();
        apply_event(
            &mut state,
            &event(1, 7, "evt_1", LifecycleEventKind::StartAttempt),
            3,
        )
        .unwrap();
        apply_event(
            &mut state,
            &event(2, 7, "evt_2", LifecycleEventKind::MarkSucceeded),
            3,
        )
        .unwrap();
        assert_eq!(state.status, ExecutionStatus::Succeeded);
        assert_eq!(
            apply_event(
                &mut state,
                &event(3, 8, "evt_3", LifecycleEventKind::StartAttempt),
                3,
            ),
            Err(LifecycleError::InvalidTransition)
        );
    }
}
