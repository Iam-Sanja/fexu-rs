//! Authentication, sessions, and permissions for Fexu.
//!
//! This crate does not depend on the admin frontend. Schema changes use the migration engine.

mod account_store;
mod anonymous;
mod config;
mod cookie;
mod error;
mod lifecycle;
mod mfa;
mod mfa_routes;
mod password;
mod policy;
mod provider;
mod recovery;
mod remote;
mod routes;
mod schema;
mod service;
mod store;
mod totp;

#[cfg(test)]
mod anonymous_tests;
#[cfg(test)]
mod lifecycle_tests;
#[cfg(test)]
mod reset_tests;
#[cfg(test)]
mod security_tests;
#[cfg(test)]
mod session_upgrade_tests;
#[cfg(test)]
mod sqlite_tests;

pub use anonymous::{AnonymousSession, BrowserActor};
pub use config::{AuthConfig, CookieSameSite, Environment};
pub use error::AuthError;
pub use lifecycle::{AccountOrigin, LocalAccountLifecycle, ProvisionedAccount};
pub use mfa::{AccountPhase, ReauthGrant, RecoverySet, SessionInfo, TotpEnrollment};
pub use password::{hash_password, validate_new_password, verify_password};
pub use policy::ObjectPolicy;
pub use provider::{IdentityProvider, PasswordProvider, VerifiedIdentity};
pub use recovery::{
    complete_reset, exchange_token, request_reset, CompleteError, IssuedExchange,
    ResetAcknowledgement,
};
pub use remote::RemoteProtocol;
pub use routes::{mount, AppState, ClientIp};
pub use schema::{
    apply_migrations, embedded_migrations, migration_file, migration_status, migrations_dir,
    schema, ADMIN_AUDIT, ADMIN_DRAFTS,
};
pub use service::{
    clean_display_name, csrf_header_name, normalize_email, AuthService, AuthUser, IssuedSession,
    PublicUser,
};
