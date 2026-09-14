use crate::MaintenanceWindow;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionWindowDecision {
    Allowed,
    TooEarly { wait_ms: u64 },
    Expired,
}

pub fn decide_execution_window(
    window: Option<&MaintenanceWindow>,
    now_unix_ms: u64,
) -> ExecutionWindowDecision {
    let Some(window) = window else {
        return ExecutionWindowDecision::Allowed;
    };

    if now_unix_ms < window.starts_at_unix_ms {
        return ExecutionWindowDecision::TooEarly {
            wait_ms: window.starts_at_unix_ms - now_unix_ms,
        };
    }
    if now_unix_ms > window.ends_at_unix_ms {
        return ExecutionWindowDecision::Expired;
    }
    ExecutionWindowDecision::Allowed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window() -> MaintenanceWindow {
        MaintenanceWindow {
            starts_at_unix_ms: 100,
            ends_at_unix_ms: 200,
        }
    }

    #[test]
    fn no_window_allows_execution() {
        assert_eq!(
            decide_execution_window(None, u64::MAX),
            ExecutionWindowDecision::Allowed
        );
    }

    #[test]
    fn execution_before_window_is_deferred() {
        assert_eq!(
            decide_execution_window(Some(&window()), 40),
            ExecutionWindowDecision::TooEarly { wait_ms: 60 }
        );
    }

    #[test]
    fn window_boundaries_are_inclusive() {
        assert_eq!(
            decide_execution_window(Some(&window()), 100),
            ExecutionWindowDecision::Allowed
        );
        assert_eq!(
            decide_execution_window(Some(&window()), 200),
            ExecutionWindowDecision::Allowed
        );
    }

    #[test]
    fn execution_after_window_fails_closed() {
        assert_eq!(
            decide_execution_window(Some(&window()), 201),
            ExecutionWindowDecision::Expired
        );
    }
}
