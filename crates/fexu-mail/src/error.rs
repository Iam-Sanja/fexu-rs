//! Fixed mail errors. Display text contains no server response or secrets.

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MailError {
    #[error("mail.disabled")]
    Disabled,
    #[error("mail.invalid")]
    Invalid,
    #[error("mail.unavailable")]
    Unavailable,
    #[error("mail.timeout")]
    Timeout,
    #[error("mail.rejected")]
    Rejected,
}
