use std::{fmt, future::Future, pin::Pin};

use thiserror::Error;
use zeroize::Zeroizing;

pub type SecretFuture<'a> =
    Pin<Box<dyn Future<Output = Result<SecretMaterial, SecretError>> + Send + 'a>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretRef {
    pub provider_id: String,
    pub secret_id: String,
    pub version: Option<String>,
}

impl SecretRef {
    pub fn validate(&self) -> Result<(), SecretError> {
        if !valid_provider_id(&self.provider_id)
            || !valid_secret_id(&self.secret_id)
            || self
                .version
                .as_deref()
                .is_some_and(|version| !valid_version(version))
        {
            return Err(SecretError::InvalidReference);
        }
        Ok(())
    }
}

pub struct SecretMaterial(Zeroizing<Vec<u8>>);

impl SecretMaterial {
    pub fn new(bytes: Vec<u8>) -> Result<Self, SecretError> {
        if bytes.is_empty() || bytes.len() > 64 * 1024 {
            return Err(SecretError::InvalidMaterial);
        }
        Ok(Self(Zeroizing::new(bytes)))
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_slice()
    }
}

impl fmt::Debug for SecretMaterial {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretMaterial(<redacted>)")
    }
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum SecretError {
    #[error("secret reference is invalid")]
    InvalidReference,
    #[error("resolved secret material is invalid")]
    InvalidMaterial,
    #[error("secret provider is unavailable")]
    Unavailable,
    #[error("secret reference was not found")]
    NotFound,
    #[error("secret resolution timed out")]
    Timeout,
}

pub trait SecretResolver: Send + Sync {
    fn resolve<'a>(&'a self, reference: &'a SecretRef) -> SecretFuture<'a>;
}

#[derive(Debug, Default)]
pub struct UnavailableSecretResolver;

impl SecretResolver for UnavailableSecretResolver {
    fn resolve<'a>(&'a self, reference: &'a SecretRef) -> SecretFuture<'a> {
        Box::pin(async move {
            reference.validate()?;
            Err(SecretError::Unavailable)
        })
    }
}

fn valid_provider_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn valid_secret_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/' | b'@')
        })
}

fn valid_version(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_are_handles_not_secret_material() {
        let reference = SecretRef {
            provider_id: "aws-secrets-manager".into(),
            secret_id: "prod/database/primary".into(),
            version: Some("v42".into()),
        };
        assert_eq!(reference.validate(), Ok(()));
        assert_eq!(
            SecretRef {
                provider_id: "aws secrets".into(),
                ..reference.clone()
            }
            .validate(),
            Err(SecretError::InvalidReference)
        );
    }

    #[test]
    fn secret_material_debug_is_always_redacted() {
        let material = SecretMaterial::new(b"postgres://user:pass@example/db".to_vec()).unwrap();
        assert_eq!(format!("{material:?}"), "SecretMaterial(<redacted>)");
        assert_eq!(material.as_bytes(), b"postgres://user:pass@example/db");
    }

    #[test]
    fn secret_material_is_bounded_and_nonempty() {
        assert!(matches!(
            SecretMaterial::new(Vec::new()),
            Err(SecretError::InvalidMaterial)
        ));
        assert!(matches!(
            SecretMaterial::new(vec![0; 64 * 1024 + 1]),
            Err(SecretError::InvalidMaterial)
        ));
    }
}
