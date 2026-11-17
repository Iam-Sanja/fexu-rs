//! Runtime configuration. The `Secure` cookie attribute does not depend on `BIND_ADDRESS`.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Environment {
    Development,
    Production,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CookieSameSite {
    Lax,
    Strict,
}

#[derive(Clone)]
pub struct AuthConfig {
    pub environment: Environment,
    pub cookie_name: String,
    /// Separate from the login cookie. Its name is fixed so an app cannot reuse the login cookie name.
    pub anonymous_cookie_name: String,
    pub cookie_secure: bool,
    pub same_site: CookieSameSite,
    pub session_ttl_secs: i64,
    pub idle_timeout_secs: i64,
    pub allowed_origins: Vec<String>,
    pub trust_proxy: bool,
    /// Immediate peers allowed to supply `X-Forwarded-For`. Empty means no peer is trusted.
    pub trusted_proxies: Vec<String>,
    pub login_max_attempts: u32,
    pub login_window_secs: i64,
    /// HMAC key for session-bound CSRF tokens.
    /// `try_from_env` loads the persistent key from `FEXU_CSRF_SECRET` or `.fexu/csrf.key`.
    /// `development` generates a new key on each call.
    pub csrf_secret: [u8; 32],
    /// AES-256-GCM key for TOTP. A separate 32-byte key independent of the CSRF secret.
    pub mfa_key: [u8; 32],
}

impl std::fmt::Debug for AuthConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AuthConfig")
            .field("environment", &self.environment)
            .field("cookie_name", &self.cookie_name)
            .field("anonymous_cookie_name", &self.anonymous_cookie_name)
            .field("cookie_secure", &self.cookie_secure)
            .field("same_site", &self.same_site)
            .field("session_ttl_secs", &self.session_ttl_secs)
            .field("idle_timeout_secs", &self.idle_timeout_secs)
            .field("allowed_origins", &self.allowed_origins)
            .field("trust_proxy", &self.trust_proxy)
            .field("trusted_proxies", &self.trusted_proxies)
            .field("login_max_attempts", &self.login_max_attempts)
            .field("login_window_secs", &self.login_window_secs)
            .field("csrf_secret", &"[redacted]")
            .field("mfa_key", &"[redacted]")
            .finish()
    }
}

impl AuthConfig {
    /// Local development defaults. Each call generates a new CSRF secret.
    /// Running processes load the persistent key through [`Self::try_from_env`].
    pub fn development() -> Self {
        Self {
            environment: Environment::Development,
            cookie_name: "fexu_session".to_owned(),
            anonymous_cookie_name: "fexu_anonymous".to_owned(),
            cookie_secure: false,
            same_site: CookieSameSite::Lax,
            session_ttl_secs: 12 * 60 * 60,
            idle_timeout_secs: 30 * 60,
            allowed_origins: vec![
                "http://127.0.0.1:3000".to_owned(),
                "http://localhost:3000".to_owned(),
            ],
            trust_proxy: false,
            trusted_proxies: Vec::new(),
            login_max_attempts: 5,
            login_window_secs: 15 * 60,
            csrf_secret: fresh_csrf_secret(),
            mfa_key: development_mfa_key(),
        }
    }

    pub fn from_env() -> Self {
        Self::try_from_env().unwrap_or_else(|error| panic!("{error}"))
    }

    /// Same values as `from_env`. Invalid values are rejected without appearing in the error.
    pub fn try_from_env() -> Result<Self, String> {
        let environment = match fexu_core::aliased_env("FEXU_ENV")?.as_deref() {
            Some("production") => Environment::Production,
            _ => Environment::Development,
        };
        let mut config = Self::development();
        config.environment = environment;
        config.cookie_secure = match fexu_core::aliased_env("FEXU_COOKIE_SECURE")? {
            Some(value) => matches!(value.as_str(), "1" | "true" | "TRUE"),
            None => environment == Environment::Production,
        };
        if let Some(origins) = fexu_core::aliased_env("FEXU_ALLOWED_ORIGINS")? {
            config.allowed_origins = origins
                .split(',')
                .map(str::trim)
                .filter(|origin| !origin.is_empty())
                .map(str::to_owned)
                .collect();
        } else if environment == Environment::Production {
            config.allowed_origins.clear();
        }
        config.csrf_secret = load_csrf_secret()?;
        config.mfa_key = load_mfa_key(environment)?;
        config.trust_proxy = matches!(
            fexu_core::aliased_env("FEXU_TRUST_PROXY")?.as_deref(),
            Some("1" | "true" | "TRUE")
        );
        if let Some(peers) = fexu_core::aliased_env("FEXU_TRUSTED_PROXIES")? {
            config.trusted_proxies = peers
                .split(',')
                .map(str::trim)
                .filter(|peer| !peer.is_empty() && *peer != "*")
                .map(str::to_owned)
                .collect();
        }
        if let Some(hours) = env_range("FEXU_SESSION_HOURS", 1, 168)? {
            config.session_ttl_secs = i64::from(hours) * 3600;
        }
        if let Some(minutes) = env_range("FEXU_IDLE_MINUTES", 1, 1440)? {
            config.idle_timeout_secs = i64::from(minutes) * 60;
        }
        if environment == Environment::Production && !config.cookie_secure {
            tracing::warn!(
                "FEXU_COOKIE_SECURE ist in Produktion aus. Session-Cookies werden ohne Secure gesendet."
            );
        }
        Ok(config)
    }

    /// File values apply only when the environment does not already provide them.
    pub fn apply_project_session(&mut self, session_hours: u32, idle_minutes: u32) {
        if !alias_present("FEXU_SESSION_HOURS") {
            self.session_ttl_secs = i64::from(session_hours) * 3600;
        }
        if !alias_present("FEXU_IDLE_MINUTES") {
            self.idle_timeout_secs = i64::from(idle_minutes) * 60;
        }
    }

    /// Ersetzt die Entwicklungsliste durch die Loopback-Origins der Prozessadresse.
    /// `FEXU_ALLOWED_ORIGINS` remains unchanged. Request Host headers are not read.
    pub fn apply_bound_address(&mut self, address: &str) {
        if alias_present("FEXU_ALLOWED_ORIGINS") {
            return;
        }
        let _ = self.with_loopback_address(address);
    }

    /// Names every browser cookie for this application.
    ///
    /// `fexu`, in any ASCII case, keeps the historical names `fexu_session`,
    /// `fexu_anonymous`, `fexu_login_csrf`, `fexu_reset_csrf` and `fexu_reset`.
    /// Any other label uses `{label}_session`, `{label}_anonymous` and the CSRF
    /// names derived from the session cookie. The label is not a `Domain`
    /// attribute and the request host is not read. An application that only
    /// replaces `cookie_name` keeps that name; login, reset and exchange cookies
    /// then follow [`Self::login_csrf_cookie`]. They are not the historical
    /// `fexu_*` CSRF cookies, so another app on the same host is not authenticated
    /// or cleared.
    pub fn with_cookie_namespace(mut self, label: &str) -> Result<Self, String> {
        let label = label.trim();
        if !valid_cookie_namespace(label) {
            return Err(
                "cookie namespace must be 1 to 32 ASCII letters or digits and start with a letter"
                    .to_owned(),
            );
        }
        if label.eq_ignore_ascii_case("fexu") {
            self.cookie_name = "fexu_session".to_owned();
            self.anonymous_cookie_name = "fexu_anonymous".to_owned();
            return Ok(self);
        }
        self.cookie_name = format!("{label}_session");
        self.anonymous_cookie_name = format!("{label}_anonymous");
        Ok(self)
    }

    /// Login double-submit cookie. The default session name keeps `fexu_login_csrf`.
    pub fn login_csrf_cookie(&self) -> String {
        namespaced_cookie(&self.cookie_name, "login_csrf")
    }

    /// Password-reset double-submit cookie. The default session name keeps `fexu_reset_csrf`.
    pub fn reset_csrf_cookie(&self) -> String {
        namespaced_cookie(&self.cookie_name, "reset_csrf")
    }

    /// Short-lived reset exchange cookie. The default session name keeps `fexu_reset`.
    pub fn reset_cookie_name(&self) -> String {
        namespaced_cookie(&self.cookie_name, "reset")
    }

    /// Setzt die erlaubten Origins auf die Loopback-Namen des Ports.
    /// Public hosts are not inferred. `0.0.0.0` and `::` are treated only as bind addresses
    /// und ergeben `127.0.0.1` sowie `localhost`, nie sich selbst als Origin.
    pub fn with_loopback_address(&mut self, address: &str) -> bool {
        let Some(origins) = loopback_http_origins(address) else {
            tracing::warn!("Die Bind-Adresse ergibt keine lokalen Login-Origins.");
            return false;
        };
        self.allowed_origins = origins;
        true
    }
}

fn valid_cookie_namespace(label: &str) -> bool {
    let mut chars = label.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (1..=32).contains(&label.len())
        && first.is_ascii_alphabetic()
        && chars.all(|char| char.is_ascii_alphanumeric())
}

/// CSRF and exchange cookie names that belong to one session cookie.
///
/// `desk_session` yields `desk_login_csrf`. A session name without that suffix
/// is kept as a prefix, so `sid` yields `sid_login_csrf`. The historical
/// `fexu_session` name yields the historical `fexu_*` cookies.
pub(crate) fn namespaced_cookie(session: &str, suffix: &str) -> String {
    if let Some(prefix) = session.strip_suffix("_session") {
        if valid_cookie_namespace(prefix) {
            return format!("{prefix}_{suffix}");
        }
    }
    format!("{session}_{suffix}")
}

/// Loopback-Origins aus der Adresse, an die der Prozess selbst bindet.
pub(crate) fn loopback_http_origins(address: &str) -> Option<Vec<String>> {
    let address = address.trim();
    if address.is_empty()
        || address.contains("://")
        || address.contains(['/', '?', '#', '@', ' ', '\\'])
    {
        return None;
    }
    let (host, port) = split_host_port(address)?;
    let port: u16 = port.parse().ok()?;
    if port == 0 {
        return None;
    }
    let host = host.trim_matches(['[', ']']);
    if !(is_loopback_host(host) || host == "0.0.0.0" || host == "::") {
        return None;
    }
    let mut origins = vec![
        format!("http://127.0.0.1:{port}"),
        format!("http://localhost:{port}"),
    ];
    if host == "::1" || host == "::" {
        origins.push(format!("http://[::1]:{port}"));
    }
    Some(origins)
}

fn split_host_port(address: &str) -> Option<(&str, &str)> {
    if let Some(rest) = address.strip_prefix('[') {
        let (host, port) = rest.split_once("]:")?;
        return Some((host, port));
    }
    let (host, port) = address.rsplit_once(':')?;
    if host.is_empty() || port.is_empty() || host.contains(':') {
        return None;
    }
    Some((host, port))
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" || host == "::1"
}

fn alias_present(name: &str) -> bool {
    !matches!(fexu_core::aliased_env(name), Ok(None))
}

fn env_range(name: &str, min: u32, max: u32) -> Result<Option<u32>, String> {
    let Some(value) = fexu_core::aliased_env(name)? else {
        return Ok(None);
    };
    match value.parse::<u32>() {
        Ok(number) if (min..=max).contains(&number) => Ok(Some(number)),
        _ => {
            tracing::warn!("{name} liegt außerhalb von {min} bis {max} und wird ignoriert.");
            Ok(None)
        }
    }
}

fn load_csrf_secret() -> Result<[u8; 32], String> {
    if let Some(value) = fexu_core::aliased_env("FEXU_CSRF_SECRET")? {
        if let Some(secret) = decode_hex_32(&value) {
            return Ok(secret);
        }
        tracing::warn!("FEXU_CSRF_SECRET muss 64 Hex-Zeichen sein. Fexu verwendet die gespeicherte Schlüsseldatei.");
    }
    let path = std::path::PathBuf::from(".fexu/csrf.key");
    if let Ok(bytes) = std::fs::read(&path) {
        if bytes.len() == 32 {
            let mut secret = [0u8; 32];
            secret.copy_from_slice(&bytes);
            return Ok(secret);
        }
    }
    let mut secret = [0u8; 32];
    getrandom::getrandom(&mut secret).expect("csrf secret");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if std::fs::write(&path, secret).is_err() {
        tracing::warn!("CSRF-Schlüsseldatei konnte nicht geschrieben werden. Der Prozessschlüssel gilt nur bis zum Neustart.");
    }
    Ok(secret)
}

fn fresh_csrf_secret() -> [u8; 32] {
    let mut secret = [0u8; 32];
    getrandom::getrandom(&mut secret).expect("csrf secret");
    secret
}

fn development_mfa_key() -> [u8; 32] {
    static KEY: std::sync::OnceLock<[u8; 32]> = std::sync::OnceLock::new();
    *KEY.get_or_init(|| {
        let mut key = [0u8; 32];
        getrandom::getrandom(&mut key).expect("mfa key");
        key
    })
}

fn load_mfa_key(environment: Environment) -> Result<[u8; 32], String> {
    match std::env::var("FEXU_MFA_KEY") {
        Ok(value) => decode_hex_32(&value)
            .ok_or_else(|| "FEXU_MFA_KEY must contain 64 hexadecimal characters.".to_owned()),
        Err(_) if environment == Environment::Production => Err(
            "FEXU_MFA_KEY is missing. Production requires 64 hexadecimal characters.".to_owned(),
        ),
        Err(_) => load_or_create_mfa_file(),
    }
}

fn load_or_create_mfa_file() -> Result<[u8; 32], String> {
    let path = std::path::PathBuf::from(".fexu/mfa.key");
    if let Ok(bytes) = std::fs::read(&path) {
        if bytes.len() == 32 {
            let mut secret = [0u8; 32];
            secret.copy_from_slice(&bytes);
            return Ok(secret);
        }
        return Err(
            "FEXU_MFA_KEY file must contain exactly 32 bytes. The contents are not shown."
                .to_owned(),
        );
    }
    let mut secret = [0u8; 32];
    getrandom::getrandom(&mut secret).expect("mfa key");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if std::fs::write(&path, secret).is_err() {
        tracing::warn!(
            "MFA-Schlüsseldatei konnte nicht geschrieben werden. Der Prozessschlüssel gilt nur bis zum Neustart."
        );
    }
    Ok(secret)
}

fn decode_hex_32(value: &str) -> Option<[u8; 32]> {
    let value = value.trim();
    if value.len() != 64 || !value.chars().all(|char| char.is_ascii_hexdigit()) {
        return None;
    }
    let mut secret = [0u8; 32];
    for (index, byte) in secret.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(secret)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_cookie_uses_the_current_framework_name() {
        let config = AuthConfig::development();
        assert_eq!(config.cookie_name, "fexu_session");
        assert_eq!(config.anonymous_cookie_name, "fexu_anonymous");
        assert_eq!(config.login_csrf_cookie(), "fexu_login_csrf");
        assert_eq!(config.reset_csrf_cookie(), "fexu_reset_csrf");
        assert_eq!(config.reset_cookie_name(), "fexu_reset");
    }

    #[test]
    fn cookie_namespace_isolates_every_browser_cookie() {
        let salon = AuthConfig::development()
            .with_cookie_namespace("salon")
            .expect("salon");
        let shop = AuthConfig::development()
            .with_cookie_namespace("Shop")
            .expect("shop");
        let historical = AuthConfig::development()
            .with_cookie_namespace("FEXU")
            .expect("historical");
        assert_eq!(salon.cookie_name, "salon_session");
        assert_eq!(salon.anonymous_cookie_name, "salon_anonymous");
        assert_eq!(salon.login_csrf_cookie(), "salon_login_csrf");
        assert_eq!(salon.reset_csrf_cookie(), "salon_reset_csrf");
        assert_eq!(salon.reset_cookie_name(), "salon_reset");
        assert_eq!(shop.cookie_name, "Shop_session");
        assert_eq!(historical.cookie_name, "fexu_session");
        assert_eq!(historical.login_csrf_cookie(), "fexu_login_csrf");
        let mut custom = AuthConfig::development();
        custom.cookie_name = "desk_session".to_owned();
        assert_eq!(custom.login_csrf_cookie(), "desk_login_csrf");
        assert_eq!(custom.anonymous_cookie_name, "fexu_anonymous");
        assert!(AuthConfig::development()
            .with_cookie_namespace("salon_shop")
            .is_err());
        assert!(AuthConfig::development()
            .with_cookie_namespace("9shop")
            .is_err());
        assert!(AuthConfig::development().with_cookie_namespace("").is_err());
    }

    #[test]
    fn loopback_origins_follow_the_bound_port() {
        assert_eq!(
            loopback_http_origins("127.0.0.1:3000"),
            Some(vec![
                "http://127.0.0.1:3000".to_owned(),
                "http://localhost:3000".to_owned()
            ])
        );
        assert_eq!(
            loopback_http_origins("localhost:4400"),
            Some(vec![
                "http://127.0.0.1:4400".to_owned(),
                "http://localhost:4400".to_owned()
            ])
        );
        assert_eq!(
            loopback_http_origins("[::1]:3000"),
            Some(vec![
                "http://127.0.0.1:3000".to_owned(),
                "http://localhost:3000".to_owned(),
                "http://[::1]:3000".to_owned(),
            ])
        );
    }

    #[test]
    fn loopback_origins_reject_public_hosts_and_port_zero() {
        let wildcard = loopback_http_origins("0.0.0.0:3017").unwrap();
        assert!(wildcard.iter().all(|origin| !origin.contains("0.0.0.0")));
        assert_eq!(wildcard[0], "http://127.0.0.1:3017");
        assert!(loopback_http_origins("203.0.113.10:3000").is_none());
        assert!(loopback_http_origins("127.0.0.1:0").is_none());
        assert!(loopback_http_origins("http://127.0.0.1:3000").is_none());
        assert!(loopback_http_origins("").is_none());
        let mut config = AuthConfig::development();
        assert!(!config.with_loopback_address("203.0.113.10:8080"));
        assert!(config
            .allowed_origins
            .iter()
            .any(|origin| origin == "http://127.0.0.1:3000"));
    }

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn restore_var(name: &str, previous: Option<String>) {
        unsafe {
            match previous {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
    }

    #[test]
    fn development_draws_a_distinct_csrf_secret_each_call() {
        let first = AuthConfig::development();
        let second = AuthConfig::development();
        assert!(
            first.csrf_secret != second.csrf_secret,
            "development CSRF secrets must differ"
        );
        assert!(first.csrf_secret.iter().any(|byte| *byte != 0));
        let rendered = format!("{first:?}");
        assert!(rendered.contains("[redacted]"));
        assert!(
            !rendered.contains(&hex_key(&first.csrf_secret)),
            "debug output contains the csrf secret"
        );
        assert!(
            !rendered.contains(&hex_key(&first.mfa_key)),
            "debug output contains the mfa key"
        );
    }

    fn hex_key(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn csrf_secret_uses_only_fexu_csrf_secret() {
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var("FEXU_CSRF_SECRET").ok();
        let secret = "ab".repeat(32);
        unsafe {
            std::env::set_var("FEXU_CSRF_SECRET", &secret);
        }
        let loaded = AuthConfig::try_from_env().expect("csrf");
        assert_eq!(
            loaded.csrf_secret,
            super::decode_hex_32(&secret).expect("hex")
        );
        assert!(!format!("{loaded:?}").contains(&secret));
        restore_var("FEXU_CSRF_SECRET", previous);
    }

    #[test]
    fn trusted_proxies_use_try_from_env() {
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var("FEXU_TRUSTED_PROXIES").ok();
        unsafe {
            std::env::set_var("FEXU_TRUSTED_PROXIES", "10.0.0.8");
        }
        let from_list = AuthConfig::try_from_env().unwrap();
        assert_eq!(from_list.trusted_proxies, vec!["10.0.0.8".to_owned()]);
        unsafe {
            std::env::set_var("FEXU_TRUSTED_PROXIES", "10.0.0.1, *");
        }
        let from_current = AuthConfig::try_from_env().unwrap();
        assert_eq!(from_current.trusted_proxies, vec!["10.0.0.1".to_owned()]);
        restore_var("FEXU_TRUSTED_PROXIES", previous);
    }

    #[test]
    fn mfa_key_is_independent_and_invalid_values_are_not_echoed() {
        let config = AuthConfig::development();
        assert_ne!(config.mfa_key, config.csrf_secret);
        let rendered = format!("{config:?}");
        assert!(rendered.contains("[redacted]"));
        assert!(!rendered.contains("fexu-dev-mfa"));
        assert!(!rendered.contains("fexu-dev-csrf"));
        let invalid = "not-a-real-mfa-key";
        assert!(decode_hex_32(invalid).is_none());
        let error = "FEXU_MFA_KEY must contain 64 hexadecimal characters.";
        assert!(!error.contains(invalid));
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous_env = std::env::var("FEXU_ENV").ok();
        let previous_mfa = std::env::var("FEXU_MFA_KEY").ok();
        let previous_csrf = std::env::var("FEXU_CSRF_SECRET").ok();
        let previous_legacy = std::env::var("FEXU_CSRF_SECRET").ok();
        unsafe {
            std::env::set_var("FEXU_ENV", "production");
            std::env::set_var("FEXU_CSRF_SECRET", "ab".repeat(32));
            std::env::remove_var("FEXU_CSRF_SECRET");
            std::env::remove_var("FEXU_MFA_KEY");
        }
        let missing = AuthConfig::try_from_env().unwrap_err();
        assert!(missing.contains("FEXU_MFA_KEY"));
        assert!(missing.contains("64"));
        unsafe {
            std::env::set_var("FEXU_MFA_KEY", invalid);
        }
        let rejected = AuthConfig::try_from_env().unwrap_err();
        assert!(rejected.contains("64 hexadecimal"));
        assert!(!rejected.contains(invalid));
        unsafe {
            std::env::set_var("FEXU_MFA_KEY", "cd".repeat(32));
        }
        let loaded = AuthConfig::try_from_env().expect("mfa key");
        assert_eq!(loaded.mfa_key, decode_hex_32(&"cd".repeat(32)).unwrap());
        assert_ne!(loaded.mfa_key, loaded.csrf_secret);
        restore_var("FEXU_ENV", previous_env);
        restore_var("FEXU_MFA_KEY", previous_mfa);
        restore_var("FEXU_CSRF_SECRET", previous_csrf);
        restore_var("FEXU_CSRF_SECRET", previous_legacy);
    }
}
