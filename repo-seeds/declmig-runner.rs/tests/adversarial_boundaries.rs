use declmig_runner::{
    admit, AdmissionError, ExecutionPolicy, ExecutionRequest, FencedExecution, MaintenanceWindow,
    MAX_ATTEMPTS, MAX_CONCURRENCY, MAX_TIMEOUT_MS,
};

fn valid() -> FencedExecution {
    FencedExecution {
        request: ExecutionRequest {
            tenant_id: "tenant_1".into(),
            project_id: "project_1".into(),
            target_id: "postgres_primary".into(),
            execution_id: "exec_1".into(),
            plan_digest: "a".repeat(64),
            idempotency_key: "migration:42".into(),
            requested_at_unix_ms: 10,
            maintenance_window: Some(MaintenanceWindow {
                starts_at_unix_ms: 0,
                ends_at_unix_ms: 100,
            }),
            policy: ExecutionPolicy {
                timeout_ms: 30_000,
                max_attempts: 3,
                max_concurrency: 1,
            },
        },
        fencing_token: 7,
    }
}

#[test]
fn timeout_boundary_is_fail_closed_and_inclusive() {
    let mut below = valid();
    below.request.policy.timeout_ms = 999;
    assert_eq!(admit(below), Err(AdmissionError::InvalidTimeout));

    let mut minimum = valid();
    minimum.request.policy.timeout_ms = 1_000;
    assert!(admit(minimum).is_ok());

    let mut maximum = valid();
    maximum.request.policy.timeout_ms = MAX_TIMEOUT_MS;
    assert!(admit(maximum).is_ok());
}

#[test]
fn attempts_boundary_rejects_zero_and_accepts_maximum() {
    let mut zero = valid();
    zero.request.policy.max_attempts = 0;
    assert_eq!(admit(zero), Err(AdmissionError::InvalidAttempts));

    let mut maximum = valid();
    maximum.request.policy.max_attempts = MAX_ATTEMPTS;
    assert!(admit(maximum).is_ok());
}

#[test]
fn concurrency_boundary_rejects_zero_and_accepts_maximum() {
    let mut zero = valid();
    zero.request.policy.max_concurrency = 0;
    assert_eq!(admit(zero), Err(AdmissionError::InvalidConcurrency));

    let mut maximum = valid();
    maximum.request.policy.max_concurrency = MAX_CONCURRENCY;
    assert!(admit(maximum).is_ok());
}

#[test]
fn inverted_maintenance_window_is_rejected() {
    let mut input = valid();
    input.request.maintenance_window = Some(MaintenanceWindow {
        starts_at_unix_ms: 101,
        ends_at_unix_ms: 100,
    });
    assert_eq!(admit(input), Err(AdmissionError::InvalidMaintenanceWindow));
}

#[test]
fn request_after_maintenance_window_is_rejected() {
    let mut input = valid();
    input.request.requested_at_unix_ms = 101;
    assert_eq!(admit(input), Err(AdmissionError::InvalidMaintenanceWindow));
}
