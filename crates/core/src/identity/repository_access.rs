//! Provider-neutral contracts for a member's repository authorization.
use super::{CredentialHash, VerifiedIdentity};
use crate::environments::EnvironmentActor;
use async_trait::async_trait;
use std::{future::Future, pin::Pin};
use uuid::Uuid;

#[derive(Clone, Copy)]
pub struct RepositoryActor {
    pub actor: EnvironmentActor,
    pub environment_id: Uuid,
    pub generation: i64,
}

/// Intentionally not Debug: a verifier must never be logged.
pub struct RepositoryAuthorizationAttempt {
    pub state_hash: CredentialHash,
    pub binding_hash: CredentialHash,
    pub verifier: String,
    pub actor: RepositoryActor,
    /// Written by persistence at begin; callers initialize this to zero.
    pub authorization_epoch: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RepositoryAccessError {
    #[error("Invalid input")]
    InvalidInput,
    #[error("Invalid authorization")]
    InvalidLogin,
    #[error("Verified provider identity does not match the signed-in user")]
    IdentityMismatch,
    #[error("Active session required")]
    Unauthorized,
    #[error("Repository access denied")]
    Forbidden,
    #[error("Provider authorization expired or revoked")]
    ProviderUnauthorized,
    #[error("Provider denied repository access")]
    ProviderForbidden,
    #[error("Provider rate limit exceeded")]
    RateLimited,
    #[error("Repository authorization conflict")]
    Conflict,
    #[error("A project directory with this name already exists")]
    DestinationExists,
    #[error("Verified primary email unavailable")]
    EmailUnavailable,
    #[error("Repository service unavailable")]
    Unavailable,
}

/// A token is passed only between trusted application components.
pub struct RepositoryAccessToken {
    access: String,
    renewal: Option<RepositoryTokenRenewal>,
}

pub struct RepositoryTokenRenewal {
    refresh: String,
    expires_at: i64,
}

impl RepositoryTokenRenewal {
    pub fn secret(&self) -> &str {
        &self.refresh
    }

    pub fn expires_at(&self) -> i64 {
        self.expires_at
    }
}

impl RepositoryAccessToken {
    pub fn new(value: String) -> Result<Self, RepositoryAccessError> {
        if value.is_empty() || value.len() > 4096 || value.chars().any(char::is_control) {
            return Err(RepositoryAccessError::InvalidLogin);
        }
        Ok(Self {
            access: value,
            renewal: None,
        })
    }

    pub fn renewable(
        access: String,
        refresh: String,
        expires_at: i64,
    ) -> Result<Self, RepositoryAccessError> {
        if expires_at <= 0 {
            return Err(RepositoryAccessError::InvalidLogin);
        }
        let mut token = Self::new(access)?;
        token.renewal = Some(RepositoryTokenRenewal {
            refresh: Self::new(refresh)?.access,
            expires_at,
        });
        Ok(token)
    }
    pub fn secret(&self) -> &str {
        &self.access
    }

    pub fn renewal(&self) -> Option<&RepositoryTokenRenewal> {
        self.renewal.as_ref()
    }
}

#[derive(Clone)]
pub struct EncryptedRepositoryToken {
    pub key_id: String,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
}

pub struct RepositoryCredential {
    pub subject: String,
    pub login: String,
    pub email: String,
    pub encrypted: EncryptedRepositoryToken,
    pub authorization_epoch: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryConnection {
    pub login: String,
    pub email: String,
}

/// A provider observation of the clone-time address, never the local Git remote.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepositoryOriginVerification {
    Matched,
    Changed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessibleRepository {
    pub id: i64,
    pub full_name: String,
    pub private: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessibleRepositoryPage {
    pub items: Vec<AccessibleRepository>,
    pub next_page: Option<u32>,
}

/// Verified project identity and the branch names of a claimed feature draft.
pub struct RepositoryDraftTarget<'a> {
    pub repository_id: i64,
    pub full_name: &'a str,
    pub base: &'a str,
    pub head: &'a str,
    pub head_oid: &'a str,
    pub operation_id: Uuid,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryDraft {
    pub url: String,
    pub base_oid: String,
    pub head_oid: String,
}

pub struct RepositoryDraftDiffTarget<'a> {
    pub repository_id: i64,
    pub full_name: &'a str,
    pub feature_id: Uuid,
    pub operation_id: Uuid,
    pub draft_url: &'a str,
}

pub struct RepositoryDraftDiff {
    pub base_oid: String,
    pub head_oid: String,
    pub patch_digest: String,
    pub patch: String,
    pub changed_paths: Vec<String>,
}

#[async_trait]
pub trait RepositoryAccessStore: Send + Sync {
    /// Capture the capability version before provider calls during sign-in.
    async fn login_credential_epoch(
        &self,
        actor: EnvironmentActor,
    ) -> Result<i64, RepositoryAccessError> {
        let _ = actor;
        Err(RepositoryAccessError::Unavailable)
    }
    /// Save only for the same live admitted session and verified external identity.
    async fn save_login_credential(
        &self,
        actor: EnvironmentActor,
        identity: &VerifiedIdentity,
        credential: RepositoryCredential,
    ) -> Result<(), RepositoryAccessError> {
        let _ = (actor, identity, credential);
        Err(RepositoryAccessError::Unavailable)
    }
    async fn begin_repository_authorization(
        &self,
        attempt: RepositoryAuthorizationAttempt,
    ) -> Result<(), RepositoryAccessError>;
    /// Consume only when state, binding, session, user and environment generation agree.
    async fn take_repository_authorization(
        &self,
        state: CredentialHash,
        binding: CredentialHash,
        session: CredentialHash,
    ) -> Result<RepositoryAuthorizationAttempt, RepositoryAccessError>;
    /// Fences both pending callbacks and results read with a stale credential.
    async fn validate_repository_epoch(
        &self,
        actor: RepositoryActor,
        epoch: i64,
    ) -> Result<(), RepositoryAccessError>;
    async fn verify_repository_identity(
        &self,
        actor: RepositoryActor,
        identity: &VerifiedIdentity,
        epoch: i64,
    ) -> Result<(), RepositoryAccessError>;
    async fn save_repository_credential(
        &self,
        actor: RepositoryActor,
        identity: &VerifiedIdentity,
        credential: RepositoryCredential,
    ) -> Result<(), RepositoryAccessError>;
    async fn repository_credential(
        &self,
        actor: RepositoryActor,
    ) -> Result<Option<RepositoryCredential>, RepositoryAccessError>;
    /// Rotate the same authorization only if the session, epoch and previous token still match.
    async fn rotate_repository_token(
        &self,
        actor: RepositoryActor,
        epoch: i64,
        previous: &EncryptedRepositoryToken,
        next: EncryptedRepositoryToken,
    ) -> Result<(), RepositoryAccessError>;
    async fn delete_repository_credential(
        &self,
        actor: RepositoryActor,
    ) -> Result<(), RepositoryAccessError>;
}

#[async_trait]
pub trait RepositoryAccessProvider: Send + Sync {
    fn authorization_url(
        &self,
        state: &str,
        challenge: &str,
    ) -> Result<String, RepositoryAccessError>;
    async fn exchange(
        &self,
        code: &str,
        verifier: &str,
    ) -> Result<(RepositoryAccessToken, VerifiedIdentity), RepositoryAccessError>;
    async fn refresh(
        &self,
        _renewal: &RepositoryTokenRenewal,
    ) -> Result<RepositoryAccessToken, RepositoryAccessError> {
        Err(RepositoryAccessError::ProviderUnauthorized)
    }
    async fn primary_email(
        &self,
        token: &RepositoryAccessToken,
        guard: &dyn RepositoryOperationGuard,
    ) -> Result<String, RepositoryAccessError>;
    async fn repositories(
        &self,
        token: &RepositoryAccessToken,
        page: u32,
    ) -> Result<AccessibleRepositoryPage, RepositoryAccessError>;
    /// Resolve the stored clone-time provider address without following redirects.
    async fn verify_clone_origin(
        &self,
        token: &RepositoryAccessToken,
        repository_id: i64,
        cloned_full_name: &str,
    ) -> Result<RepositoryOriginVerification, RepositoryAccessError>;
    /// A successful false result requires a complete, bounded provider listing.
    async fn public_key_registered(
        &self,
        token: &RepositoryAccessToken,
        public_key: &str,
        guard: &dyn RepositoryOperationGuard,
    ) -> Result<bool, RepositoryAccessError>;
    async fn ensure_public_key(
        &self,
        token: &RepositoryAccessToken,
        public_key: &str,
        guard: &dyn RepositoryOperationGuard,
    ) -> Result<(), RepositoryAccessError>;
    /// A successful absence requires checking the exact head across all PR states.
    async fn find_draft(
        &self,
        _token: &RepositoryAccessToken,
        _target: &RepositoryDraftTarget<'_>,
        _guard: &dyn RepositoryOperationGuard,
    ) -> Result<Option<RepositoryDraft>, RepositoryAccessError> {
        Err(RepositoryAccessError::Unavailable)
    }
    async fn create_draft(
        &self,
        _token: &RepositoryAccessToken,
        _target: &RepositoryDraftTarget<'_>,
        _title: &str,
        _guard: &dyn RepositoryOperationGuard,
    ) -> Result<RepositoryDraft, RepositoryAccessError> {
        Err(RepositoryAccessError::Unavailable)
    }
    /// Read the current diff for a PR identified by a durable Factory receipt.
    async fn draft_diff(
        &self,
        _token: &RepositoryAccessToken,
        _target: &RepositoryDraftDiffTarget<'_>,
        _guard: &dyn RepositoryOperationGuard,
    ) -> Result<RepositoryDraftDiff, RepositoryAccessError> {
        Err(RepositoryAccessError::Unavailable)
    }
}

/// Revalidate a bound authorization immediately before each provider request.
pub trait RepositoryOperationGuard: Send + Sync {
    fn check(&self)
    -> Pin<Box<dyn Future<Output = Result<(), RepositoryAccessError>> + Send + '_>>;
}

pub trait RepositoryTokenCipher: Send + Sync {
    fn seal(
        &self,
        user_id: Uuid,
        token: &RepositoryAccessToken,
    ) -> Result<EncryptedRepositoryToken, RepositoryAccessError>;
    fn open(
        &self,
        user_id: Uuid,
        token: &EncryptedRepositoryToken,
    ) -> Result<RepositoryAccessToken, RepositoryAccessError>;
}

pub struct RepositoryPublicKey {
    pub public_key: String,
    /// True only when durable evidence matches the current public key.
    pub registered: bool,
}

#[async_trait]
pub trait RepositoryPublicKeySource: Send + Sync {
    /// Implementations must resolve only the owned, fenced environment; no browser paths.
    async fn public_key_status(
        &self,
        actor: RepositoryActor,
    ) -> Result<RepositoryPublicKey, RepositoryAccessError>;
    /// Write evidence only if the current key still matches the expected key.
    async fn mark_registered(
        &self,
        actor: RepositoryActor,
        expected_key: &str,
    ) -> Result<(), RepositoryAccessError>;
    /// Call only after complete provider enumeration confirms a previously registered key is absent.
    async fn rotate_missing_key(
        &self,
        actor: RepositoryActor,
        expected_key: &str,
    ) -> Result<RepositoryPublicKey, RepositoryAccessError>;
}
