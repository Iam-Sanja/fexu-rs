//! Local password account lifecycle.
//!
//! This is the first provider boundary. It creates, suspends, restores and
//! signs out local accounts. It does not write cookies or check CSRF.
//! OIDC, OAuth and passkeys stay unimplemented in `RemoteProtocol`.

use crate::error::AuthError;
use crate::mfa::AccountPhase;
use crate::service::{normalize_email, unix_now, AuthService, PublicUser};
use crate::store;
use async_trait::async_trait;

/// Where the account credential comes from. Only the local password exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountOrigin {
    LocalPassword,
}

impl AccountOrigin {
    /// Remote assertions are not accepted. A future protocol needs its own review.
    pub const fn accepts_remote_assertion(self) -> bool {
        match self {
            Self::LocalPassword => false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProvisionedAccount {
    pub user: PublicUser,
    pub origin: AccountOrigin,
}

/// Operations for a local password account. Implementations must not issue a session.
#[async_trait]
pub trait LocalAccountLifecycle: Send + Sync {
    async fn provision_local(
        &self,
        email: &str,
        password: &str,
        is_staff: bool,
        is_superuser: bool,
    ) -> Result<ProvisionedAccount, AuthError>;

    async fn suspend_local(&self, email: &str) -> Result<AccountPhase, AuthError>;

    async fn restore_local(&self, email: &str) -> Result<AccountPhase, AuthError>;

    async fn revoke_local_sessions(&self, email: &str) -> Result<(), AuthError>;
}

#[async_trait]
impl LocalAccountLifecycle for AuthService {
    async fn provision_local(
        &self,
        email: &str,
        password: &str,
        is_staff: bool,
        is_superuser: bool,
    ) -> Result<ProvisionedAccount, AuthError> {
        let user = self
            .create_user(email, password, is_staff, is_superuser)
            .await?;
        Ok(ProvisionedAccount {
            user,
            origin: AccountOrigin::LocalPassword,
        })
    }

    async fn suspend_local(&self, email: &str) -> Result<AccountPhase, AuthError> {
        self.set_user_active(email, false).await?;
        Ok(AccountPhase::Disabled)
    }

    async fn restore_local(&self, email: &str) -> Result<AccountPhase, AuthError> {
        self.set_user_active(email, true).await?;
        Ok(AccountPhase::Active)
    }

    async fn revoke_local_sessions(&self, email: &str) -> Result<(), AuthError> {
        let email = normalize_email(email)?;
        let user = store::find_user_by_email(self.database(), &email)
            .await?
            .ok_or_else(|| AuthError::Validation("user was not found".into()))?;
        store::revoke_user_sessions(self.database(), user.id, unix_now()?).await
    }
}
