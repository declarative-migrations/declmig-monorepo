use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod lifecycle;
pub mod secrets;
pub mod window;

pub const MAX_INPUT_BYTES: u64 = 64 * 1024;
pub const MAX_TIMEOUT_MS: u64 = 60 * 60 * 1000;
pub const MAX_ATTEMPTS: u8 = 32;
pub const MAX_CONCURRENCY: u16 = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionPolicy {
    pub timeout_ms: u64,
    pub max_attempts: u8,
    pub max_concurrency: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaintenanceWindow {
    pub starts_at_unix_ms: u64,
    pub ends_at_unix_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionRequest {
    pub tenant_id: String,
    pub project_id: String,
    pub target_id: String,
    pub execution_id: String,
    pub plan_digest: String,
    pub idempotency_key: String,
    pub requested_at_unix_ms: u64,
    pub maintenance_window: Option<MaintenanceWindow>,
    pub policy: ExecutionPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FencedExecution {
    pub request: ExecutionRequest,
    pub fencing_token: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdmissionReceipt {
    pub accepted: bool,
    pub execution_id: String,
    pub plan_digest: String,
    pub fencing_token: u64,
    pub side_effects_started: bool,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AdmissionError {
    #[error("invalid identifier: {0}")]
    InvalidIdentifier(&'static str),
    #[error("plan_digest must be 64 lowercase hexadecimal characters")]
    InvalidPlanDigest,
    #[error("fencing_token must be non-zero")]
    InvalidFencingToken,
    #[error("timeout_ms must be between 1000 and {MAX_TIMEOUT_MS}")]
    InvalidTimeout,
    #[error("max_attempts must be between 1 and {MAX_ATTEMPTS}")]
    InvalidAttempts,
    #[error("max_concurrency must be between 1 and {MAX_CONCURRENCY}")]
    InvalidConcurrency,
    #[error("maintenance window is invalid")]
    InvalidMaintenanceWindow,
}

pub fn admit(input: FencedExecution) -> Result<AdmissionReceipt, AdmissionError> {
    validate_request(&input.request)?;
    if input.fencing_token == 0 {
        return Err(AdmissionError::InvalidFencingToken);
    }

    Ok(AdmissionReceipt {
        accepted: true,
        execution_id: input.request.execution_id,
        plan_digest: input.request.plan_digest,
        fencing_token: input.fencing_token,
        side_effects_started: false,
    })
}

pub fn validate_request(request: &ExecutionRequest) -> Result<(), AdmissionError> {
    validate_id("tenant_id", &request.tenant_id)?;
    validate_id("project_id", &request.project_id)?;
    validate_id("target_id", &request.target_id)?;
    validate_id("execution_id", &request.execution_id)?;
    validate_id("idempotency_key", &request.idempotency_key)?;

    if request.plan_digest.len() != 64
        || !request
            .plan_digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(AdmissionError::InvalidPlanDigest);
    }

    if !(1_000..=MAX_TIMEOUT_MS).contains(&request.policy.timeout_ms) {
        return Err(AdmissionError::InvalidTimeout);
    }
    if !(1..=MAX_ATTEMPTS).contains(&request.policy.max_attempts) {
        return Err(AdmissionError::InvalidAttempts);
    }
    if !(1..=MAX_CONCURRENCY).contains(&request.policy.max_concurrency) {
        return Err(AdmissionError::InvalidConcurrency);
    }

    if let Some(window) = &request.maintenance_window {
        if window.starts_at_unix_ms > window.ends_at_unix_ms
            || request.requested_at_unix_ms > window.ends_at_unix_ms
        {
            return Err(AdmissionError::InvalidMaintenanceWindow);
        }
    }

    Ok(())
}

fn validate_id(field: &'static str, value: &str) -> Result<(), AdmissionError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(AdmissionError::InvalidIdentifier(field));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn valid_request() -> FencedExecution {
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
    fn deterministic_replay_is_side_effect_free() {
        let first = admit(valid_request()).unwrap();
        let second = admit(valid_request()).unwrap();
        assert_eq!(first, second);
        assert!(!first.side_effects_started);
    }

    #[test]
    fn stale_or_missing_fencing_is_rejected() {
        let mut input = valid_request();
        input.fencing_token = 0;
        assert_eq!(admit(input), Err(AdmissionError::InvalidFencingToken));
    }

    #[test]
    fn secret_and_remote_exec_fields_fail_closed() {
        for forbidden in [
            "password",
            "database_url",
            "dsn",
            "bearer_token",
            "private_key",
            "command",
        ] {
            let mut value = serde_json::to_value(valid_request()).unwrap();
            value
                .as_object_mut()
                .unwrap()
                .insert(forbidden.into(), json!("secret-or-command"));
            assert!(
                serde_json::from_value::<FencedExecution>(value).is_err(),
                "accepted forbidden field {forbidden}"
            );
        }
    }

    #[test]
    fn bounds_are_enforced() {
        let mut input = valid_request();
        input.request.policy.max_attempts = MAX_ATTEMPTS + 1;
        assert_eq!(admit(input), Err(AdmissionError::InvalidAttempts));

        let mut input = valid_request();
        input.request.policy.max_concurrency = MAX_CONCURRENCY + 1;
        assert_eq!(admit(input), Err(AdmissionError::InvalidConcurrency));
    }

    #[test]
    fn digest_and_identity_are_canonical() {
        let mut input = valid_request();
        input.request.plan_digest = "A".repeat(64);
        assert_eq!(admit(input), Err(AdmissionError::InvalidPlanDigest));

        let mut input = valid_request();
        input.request.tenant_id = "tenant ü".into();
        assert_eq!(
            admit(input),
            Err(AdmissionError::InvalidIdentifier("tenant_id"))
        );
    }
}
