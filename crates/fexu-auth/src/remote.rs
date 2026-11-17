//! OIDC, OAuth, and passkeys are not implemented.
//!
//! Limitations are documented in `docs/auth-threat-model.md`. No method
//! reports an external login as successful.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteProtocol {
    Oidc,
    OAuthAuthorizationCode,
    Passkey,
}

impl RemoteProtocol {
    pub const fn is_implemented(self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::RemoteProtocol;

    #[test]
    fn remote_protocols_are_not_implemented() {
        for protocol in [
            RemoteProtocol::Oidc,
            RemoteProtocol::OAuthAuthorizationCode,
            RemoteProtocol::Passkey,
        ] {
            assert!(!protocol.is_implemented());
        }
    }
}
