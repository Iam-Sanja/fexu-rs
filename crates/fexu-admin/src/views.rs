//! Registered admin pages. Page paths come from the registry, never from arbitrary URL segments.

use crate::registry::AdminRegistry;
use fexu_auth::AuthUser;
use fexu_db::driver::DatabaseConnection;
use serde_json::{Map, Value};
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

pub const RESERVED_KEYS: &[&str] = &[
    "admin",
    "api",
    "assets",
    "account",
    "audit",
    "boards",
    "directory",
    "login",
    "logout",
    "models",
    "pages",
    "preferences",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionError {
    pub code: &'static str,
    pub detail: Cow<'static, str>,
}

impl ExtensionError {
    pub const fn new(code: &'static str, detail: &'static str) -> Self {
        Self {
            code,
            detail: Cow::Borrowed(detail),
        }
    }

    pub fn detailed(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: Cow::Owned(detail.into()),
        }
    }
}

impl std::fmt::Display for ExtensionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.detail)
    }
}

impl std::error::Error for ExtensionError {}

/// A file field from a multipart admin form. The bytes are not stored by the framework.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadedFile {
    pub field: String,
    pub filename: String,
    /// Browser-supplied Content-Type. Do not use it as proof of the file contents.
    pub declared_type: Option<String>,
    pub bytes: Vec<u8>,
}

impl UploadedFile {
    pub fn size(&self) -> usize {
        self.bytes.len()
    }
}

pub struct ViewRequest<'a> {
    pub user: &'a AuthUser,
    pub db: &'a DatabaseConnection,
    pub locale: &'a str,
    /// Checked GET parameters. Parsing them does not write to the database.
    pub query: &'a ViewQuery,
    pub form: &'a HashMap<String, String>,
    /// Empty for GET and for URL-encoded forms.
    pub uploads: &'a [UploadedFile],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewPage {
    pub data: Map<String, Value>,
    pub notice: String,
    pub alert: String,
}

impl ViewPage {
    pub fn new(data: Map<String, Value>) -> Self {
        Self {
            data,
            notice: String::new(),
            alert: String::new(),
        }
    }
}

/// HTTP result of a custom admin page. The framework maps each variant itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewOutcome {
    Page(ViewPage),
    /// Canonical internal admin location. The dispatcher checks it again.
    Redirect(String),
    NotFound,
    Conflict,
    Invalid,
}

impl From<ViewPage> for ViewOutcome {
    fn from(page: ViewPage) -> Self {
        Self::Page(page)
    }
}

impl ViewOutcome {
    pub fn page(page: ViewPage) -> Self {
        Self::Page(page)
    }

    /// Accepts only a canonical `/admin` path, optionally with a checked query.
    pub fn redirect(target: &str) -> Result<Self, ExtensionError> {
        match internal_admin_location(target) {
            Some(location) => Ok(Self::Redirect(location)),
            None => Err(ExtensionError::new(
                "extension.redirect",
                "Die Weiterleitung muss auf einen internen Admin-Pfad zeigen.",
            )),
        }
    }

    pub fn not_found() -> Self {
        Self::NotFound
    }

    pub fn conflict() -> Self {
        Self::Conflict
    }

    pub fn invalid() -> Self {
        Self::Invalid
    }
}

/// Read-only GET parameters for a custom admin page.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ViewQuery {
    values: Vec<(String, String)>,
}

impl ViewQuery {
    pub fn parse(raw: Option<&str>) -> Result<Self, ExtensionError> {
        let Some(raw) = raw.filter(|value| !value.is_empty()) else {
            return Ok(Self { values: Vec::new() });
        };
        if raw.len() > 256
            || raw
                .chars()
                .any(|char| char.is_control() || char.is_whitespace())
        {
            return Err(invalid_query());
        }
        let mut values = Vec::new();
        for part in raw.split('&') {
            if values.len() == 8 || part.is_empty() {
                return Err(invalid_query());
            }
            let Some((key, value)) = part.split_once('=') else {
                return Err(invalid_query());
            };
            if !query_key(key) || values.iter().any(|(existing, _)| existing == key) {
                return Err(invalid_query());
            }
            if nested_separator(value) {
                return Err(invalid_query());
            }
            let decoded = strict_decode(value).ok_or_else(invalid_query)?;
            if !query_value(&decoded) {
                return Err(invalid_query());
            }
            values.push((key.to_owned(), decoded));
        }
        Ok(Self { values })
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.values
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    /// `YYYY-MM-DD` only. A missing key is `Ok(None)`. A present but unusable value is 422.
    pub fn date(&self, name: &str) -> Result<Option<chrono::NaiveDate>, ExtensionError> {
        let Some(value) = self.get(name) else {
            return Ok(None);
        };
        if value.len() != 10 {
            return Err(invalid_query());
        }
        chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
            .map(Some)
            .map_err(|_| invalid_query())
    }

    fn encode(&self) -> String {
        self.values
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join("&")
    }
}

#[async_trait::async_trait]
pub trait ViewHandler: Send + Sync {
    async fn handle(&self, request: &ViewRequest<'_>) -> Result<ViewOutcome, ExtensionError>;
}

#[derive(Clone)]
pub struct AdminView {
    key: &'static str,
    title_de: &'static str,
    title_en: &'static str,
    permission: &'static str,
    navigation: bool,
    template: &'static str,
    get: Option<Arc<dyn ViewHandler>>,
    post: Option<Arc<dyn ViewHandler>>,
}

impl AdminView {
    pub fn new(key: &'static str) -> Self {
        Self {
            key,
            title_de: key,
            title_en: key,
            permission: "",
            navigation: false,
            template: "custom.html",
            get: None,
            post: None,
        }
    }

    pub fn titles(mut self, german: &'static str, english: &'static str) -> Self {
        self.title_de = german;
        self.title_en = english;
        self
    }

    /// Full permission code, for example `orders.read`. Fexu checks it before calling the handler.
    pub fn permission(mut self, code: &'static str) -> Self {
        self.permission = code;
        self
    }

    pub fn navigation(mut self, enabled: bool) -> Self {
        self.navigation = enabled;
        self
    }

    /// Name einer registrierten Vorlage. Kein Dateipfad.
    pub fn template(mut self, name: &'static str) -> Self {
        self.template = name;
        self
    }

    pub fn on_get(mut self, handler: impl ViewHandler + 'static) -> Self {
        self.get = Some(Arc::new(handler));
        self
    }

    pub fn on_post(mut self, handler: impl ViewHandler + 'static) -> Self {
        self.post = Some(Arc::new(handler));
        self
    }

    pub(crate) fn key(&self) -> &'static str {
        self.key
    }

    pub(crate) fn title(&self, locale: &str) -> &'static str {
        if locale == "de" {
            self.title_de
        } else {
            self.title_en
        }
    }

    pub(crate) fn permission_code(&self) -> &'static str {
        self.permission
    }

    pub(crate) fn in_navigation(&self) -> bool {
        self.navigation
    }

    pub(crate) fn template_name(&self) -> &'static str {
        self.template
    }

    pub(crate) fn get_handler(&self) -> Option<&Arc<dyn ViewHandler>> {
        self.get.as_ref()
    }

    pub(crate) fn post_handler(&self) -> Option<&Arc<dyn ViewHandler>> {
        self.post.as_ref()
    }
}

pub(crate) fn valid_key(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some(char) if char.is_ascii_lowercase())
        && chars.all(|char| char.is_ascii_lowercase() || char.is_ascii_digit() || char == '_')
        && value.len() <= 40
}

pub(crate) fn valid_permission(value: &str) -> bool {
    let Some((resource, action)) = value.split_once('.') else {
        return false;
    };
    valid_key(resource) && valid_key(action) && !value.contains("..")
}

pub(crate) fn valid_label(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty()
        && trimmed.len() <= 80
        && !trimmed
            .chars()
            .any(|char| char.is_control() || matches!(char, '<' | '>' | '"' | '\''))
}

pub(crate) fn check_view(view: &AdminView, templates: &[&str]) -> Result<(), ExtensionError> {
    if !valid_key(view.key) || RESERVED_KEYS.contains(&view.key) {
        return Err(ExtensionError::new(
            "extension.route",
            "The page name is empty, reserved, or contains invalid characters.",
        ));
    }
    if !valid_permission(view.permission) {
        return Err(ExtensionError::new(
            "extension.permission",
            "A custom page requires a resource.action permission code.",
        ));
    }
    if !valid_label(view.title_de) || !valid_label(view.title_en) {
        return Err(ExtensionError::new(
            "extension.route",
            "The page title is empty or contains control characters.",
        ));
    }
    if view.get.is_none() {
        return Err(ExtensionError::new(
            "extension.route",
            "A custom page requires a GET handler.",
        ));
    }
    if !templates.contains(&view.template) && view.template != "custom.html" {
        return Err(ExtensionError::new(
            "extension.template",
            "The page template is not registered.",
        ));
    }
    Ok(())
}

impl AdminRegistry {
    pub(crate) fn view_by_key(&self, key: &str) -> Option<&AdminView> {
        self.views().iter().find(|view| view.key() == key)
    }
}

fn invalid_query() -> ExtensionError {
    ExtensionError::new("extension.query", "The filter parameters are invalid.")
}

fn query_key(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some(char) if char.is_ascii_lowercase())
        && chars.all(|char| char.is_ascii_lowercase() || char.is_ascii_digit() || char == '_')
        && value.len() <= 32
}

fn query_value(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && !value.contains("..")
        && value
            .chars()
            .all(|char| char.is_ascii_alphanumeric() || matches!(char, '-' | '_' | '.'))
}

/// Canonical `/admin` location. Encoded separators, schemes and off-admin paths are rejected.
pub(crate) fn internal_admin_location(target: &str) -> Option<String> {
    if target.is_empty()
        || target.len() > 200
        || target
            .chars()
            .any(|char| char.is_control() || char.is_whitespace())
        || target.contains(['\\', '#', '@'])
        || nested_separator(target)
    {
        return None;
    }
    let decoded = strict_decode(target)?;
    let (path, query) = match decoded.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (decoded.as_str(), None),
    };
    if !admin_path(path) {
        return None;
    }
    let Some(query) = query else {
        return Some(path.to_owned());
    };
    let parsed = ViewQuery::parse(Some(query)).ok()?;
    Some(format!("{path}?{}", parsed.encode()))
}

fn admin_path(path: &str) -> bool {
    let Some(rest) = path.strip_prefix("/admin") else {
        return false;
    };
    if rest.is_empty() {
        return true;
    }
    let Some(rest) = rest.strip_prefix('/') else {
        return false;
    };
    !rest.is_empty()
        && rest.split('/').all(|segment| {
            !segment.is_empty()
                && segment.len() <= 64
                && segment
                    .chars()
                    .all(|char| char.is_ascii_alphanumeric() || matches!(char, '_' | '-'))
        })
}

fn nested_separator(value: &str) -> bool {
    let mut current = value.to_owned();
    for _ in 0..4 {
        if encoded_separator(&current) {
            return true;
        }
        if !current.as_bytes().contains(&b'%') {
            return false;
        }
        let Some(decoded) = strict_decode(&current) else {
            return true;
        };
        if decoded == current {
            return false;
        }
        current = decoded;
    }
    true
}

fn encoded_separator(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut index = 0;
    while index + 2 < bytes.len() {
        if bytes[index] == b'%' {
            let hex = &bytes[index + 1..index + 3];
            if hex.eq_ignore_ascii_case(b"2e")
                || hex.eq_ignore_ascii_case(b"2f")
                || hex.eq_ignore_ascii_case(b"5c")
            {
                return true;
            }
        }
        index += 1;
    }
    false
}

fn strict_decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut output = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return None;
            }
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok()?;
            let byte = u8::from_str_radix(hex, 16).ok()?;
            if byte == 0 || byte == b'\\' || byte.is_ascii_control() {
                return None;
            }
            output.push(byte);
            index += 3;
            continue;
        }
        output.push(bytes[index]);
        index += 1;
    }
    String::from_utf8(output).ok()
}

#[cfg(test)]
mod tests {
    use super::{AdminView, ExtensionError, ViewOutcome, ViewQuery};
    use crate::registry::AdminRegistry;

    #[test]
    fn reserved_routes_and_unsafe_templates_fail() {
        let reserved = AdminRegistry::new().view(AdminView::new("models"));
        assert!(matches!(
            reserved,
            Err(ExtensionError {
                code: "extension.route",
                ..
            })
        ));
        let traversal =
            AdminRegistry::new().template("../base.html", "{% extends \"base.html\" %}");
        assert!(traversal.is_err());
        let unescaped = AdminRegistry::new().template(
            "reports.html",
            "{% extends \"base.html\" %}{{ body | safe }}",
        );
        assert!(unescaped.is_err());
    }

    #[test]
    fn query_reads_a_date_and_rejects_hostile_parameters() {
        let query = ViewQuery::parse(Some("date=2026-01-02&status=open")).unwrap();
        assert_eq!(
            query.date("date").unwrap().unwrap().to_string(),
            "2026-01-02"
        );
        assert_eq!(query.get("status"), Some("open"));
        assert!(query.date("missing").unwrap().is_none());
        assert!(ViewQuery::parse(None).unwrap().get("date").is_none());
        assert!(ViewQuery::parse(Some("date=2026-99-99"))
            .unwrap()
            .date("date")
            .is_err());
        for raw in [
            "date=%3Cscript%3E",
            "date=%252e%252e",
            "date=..",
            "next=https://evil.example",
            "date=2026-01-02&date=2026-01-03",
            "Date=2026-01-02",
            "date=",
            "date",
        ] {
            assert!(ViewQuery::parse(Some(raw)).is_err(), "{raw}");
        }
    }

    #[test]
    fn redirect_stays_on_an_admin_path() {
        assert_eq!(
            ViewOutcome::redirect("/admin/pages/reports?date=2026-01-02").unwrap(),
            ViewOutcome::Redirect("/admin/pages/reports?date=2026-01-02".to_owned())
        );
        assert_eq!(
            super::internal_admin_location("/admin"),
            Some("/admin".to_owned())
        );
        for target in [
            "https://evil.example/admin",
            "//evil.example/admin",
            "/admin/../secret",
            "/admin/pages/%2e%2e/login",
            "/admin/pages/%252e%252e/login",
            "/api/secret",
            "/admin/pages/reports?next=https://evil.example",
            "\\admin",
            "/admin/pages/reports#fragment",
        ] {
            assert!(ViewOutcome::redirect(target).is_err(), "{target}");
            assert!(super::internal_admin_location(target).is_none(), "{target}");
        }
    }
}
