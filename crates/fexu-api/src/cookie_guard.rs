//! Cookie-authenticated writes require CSRF validation before a handler runs.
//!
//! `App::require_browser_writes` wraps [`enforce`] with [`require_browser`]. Both call
//! [`gate`]. Without the configured session or anonymous cookie, `enforce` passes the
//! request to the handler, except for dedicated login/reset forms when
//! [`require_browser`] is enabled. In that case the configured login or reset CSRF
//! cookie is sufficient proof even from a fresh cookie jar. The historical names are
//! `fexu_session`, `fexu_anonymous`, `fexu_login_csrf` and `fexu_reset_csrf`. Another
//! application's cookies are not read. Other write routes still require a session cookie.
//! `Authorization: Bearer` is not validated here. An expired or unknown session cookie
//! neither replaces nor blocks the dedicated login proof.
//! `Origin: null` is not generally accepted. The existing exception requires
//! `Sec-Fetch-Site: same-origin`. The Host and `X-Forwarded-*` headers are not read here.

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use fexu_auth::{csrf_header_name, AppState, AuthError};
use fexu_core::{problem_json, FexuError};
use http_body_util::{BodyExt, Limited};

use super::ApiError;

const BODY_CAP: usize = 8 * 1024 * 1024;
const DEFAULT_SESSION_COOKIES: [&str; 2] = ["fexu_session", "fexu_anonymous"];

pub async fn enforce(State(state): State<AppState>, request: Request, next: Next) -> Response {
    gate(state, request, next, false).await
}

/// Restricts writes to requests with valid browser CSRF proof. Missing session cookies and unchecked
/// `Authorization: Bearer` values are insufficient.
pub async fn require_browser(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    gate(state, request, next, true).await
}

async fn gate(state: AppState, request: Request, next: Next, browser_required: bool) -> Response {
    if request.method().is_safe() {
        return next.run(request).await;
    }
    let path = request.uri().path().to_owned();
    let dedicated = dedicated_csrf_cookie(state.auth.as_ref(), &path);
    let protected = match state.auth.as_ref() {
        Some(auth) => auth.request_has_session_cookie(request.headers()),
        None => default_session_cookie(request.headers()),
    };
    if !protected {
        let login_proof_without_session = browser_required && dedicated.is_some();
        if !login_proof_without_session {
            if browser_required {
                return reject(AuthError::CsrfRejected, request.headers());
            }
            return next.run(request).await;
        }
    }
    let Some(auth) = state.auth.clone() else {
        return reject(AuthError::CsrfRejected, request.headers());
    };
    let header_token = header_csrf(request.headers());
    let (request, field) = if header_token.is_empty() || dedicated.is_some() {
        match body_field(request).await {
            Ok(pair) => pair,
            Err(BodyRead::TooLarge) => {
                return problem(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "Payload too large",
                    "request body is too large",
                );
            }
            Err(BodyRead::Unreadable) => {
                return problem(
                    StatusCode::BAD_REQUEST,
                    "Bad request",
                    "request body could not be read",
                );
            }
        }
    } else {
        (request, String::new())
    };
    if let Some(cookie_name) = dedicated {
        if dedicated_proof(
            &auth,
            request.headers(),
            request.method(),
            &cookie_name,
            path == "/admin/login",
            &header_token,
            &field,
        ) {
            return next.run(request).await;
        }
        return reject(AuthError::CsrfRejected, request.headers());
    }
    match auth
        .guard_cookie_write(request.headers(), request.method(), &field)
        .await
    {
        Ok(_) => next.run(request).await,
        Err(error) => reject(error, request.headers()),
    }
}

fn dedicated_csrf_cookie(auth: Option<&fexu_auth::AuthService>, path: &str) -> Option<String> {
    let config = auth.map(|auth| auth.config());
    match path {
        "/admin/login" => Some(
            config
                .map(|config| config.login_csrf_cookie())
                .unwrap_or_else(|| "fexu_login_csrf".to_owned()),
        ),
        "/auth/password-reset" | "/auth/password-reset/choose" => Some(
            config
                .map(|config| config.reset_csrf_cookie())
                .unwrap_or_else(|| "fexu_reset_csrf".to_owned()),
        ),
        _ => None,
    }
}

fn dedicated_proof(
    auth: &fexu_auth::AuthService,
    headers: &HeaderMap,
    method: &Method,
    cookie_name: &str,
    login_form: bool,
    header_token: &str,
    field: &str,
) -> bool {
    if auth.origin_allowed(headers, method).is_err() {
        return false;
    }
    let Some(expected) = cookie_value(headers, cookie_name) else {
        return false;
    };
    if login_form && !valid_login_csrf(&expected) {
        return false;
    }
    (!field.is_empty() && constant_eq(field, &expected))
        || (!header_token.is_empty() && constant_eq(header_token, &expected))
}

fn valid_login_csrf(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|char| char.is_ascii_hexdigit())
}

fn constant_eq(left: &str, right: &str) -> bool {
    let (left, right) = (left.as_bytes(), right.as_bytes());
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |diff, (left, right)| diff | (left ^ right))
        == 0
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers.get_all(header::COOKIE).iter().find_map(|value| {
        let raw = value.to_str().ok()?;
        raw.split(';').find_map(|part| {
            let (key, token) = part.trim().split_once('=')?;
            (key.trim() == name).then(|| token.trim().to_owned())
        })
    })
}

fn default_session_cookie(headers: &HeaderMap) -> bool {
    DEFAULT_SESSION_COOKIES
        .iter()
        .any(|name| cookie_named(headers, name))
}

fn cookie_named(headers: &HeaderMap, name: &str) -> bool {
    let Some(raw) = headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    raw.split(';').any(|part| {
        part.trim()
            .split_once('=')
            .is_some_and(|(key, _)| key.trim() == name)
    })
}

fn header_csrf(headers: &HeaderMap) -> String {
    headers
        .get(csrf_header_name())
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .trim()
        .to_owned()
}

enum BodyRead {
    TooLarge,
    Unreadable,
}

async fn body_field(request: Request) -> Result<(Request, String), BodyRead> {
    let (parts, body) = request.into_parts();
    let content_type = parts
        .headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let bytes = match Limited::new(body, BODY_CAP).collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(error) => {
            return Err(
                if error
                    .downcast_ref::<http_body_util::LengthLimitError>()
                    .is_some()
                {
                    BodyRead::TooLarge
                } else {
                    BodyRead::Unreadable
                },
            );
        }
    };
    let field = csrf_field(&content_type, &bytes);
    Ok((Request::from_parts(parts, Body::from(bytes)), field))
}

fn csrf_field(content_type: &str, body: &[u8]) -> String {
    let mime = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if mime == "application/json" {
        return String::new();
    }
    if mime == "multipart/form-data" {
        return boundary(content_type)
            .map(|marker| multipart_csrf(body, &marker))
            .unwrap_or_default();
    }
    if mime == "application/x-www-form-urlencoded" || mime.is_empty() {
        return form_csrf(body);
    }
    String::new()
}

fn boundary(content_type: &str) -> Option<String> {
    content_type.split(';').find_map(|part| {
        let value = part.trim().strip_prefix("boundary=")?;
        let value = value.trim().trim_matches('"');
        if value.is_empty()
            || value.len() > 70
            || value
                .chars()
                .any(|char| char.is_control() || char.is_whitespace())
        {
            return None;
        }
        Some(value.to_owned())
    })
}

fn form_csrf(body: &[u8]) -> String {
    let Ok(text) = std::str::from_utf8(body) else {
        return String::new();
    };
    let mut token = String::new();
    for pair in text.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if decode(key) == "csrf_token" && token.is_empty() {
            token = bounded(decode(value));
        }
    }
    token
}

fn multipart_csrf(body: &[u8], marker: &str) -> String {
    let delimiter = format!("--{marker}").into_bytes();
    let mut rest = body;
    while let Some(start) = find_slice(rest, &delimiter) {
        rest = &rest[start + delimiter.len()..];
        if rest.starts_with(b"--") {
            break;
        }
        let rest_part = rest.strip_prefix(b"\r\n").unwrap_or(rest);
        let Some(split) = find_slice(rest_part, b"\r\n\r\n") else {
            continue;
        };
        let headers = &rest_part[..split];
        let content = &rest_part[split + 4..];
        if !is_csrf_part(headers) {
            continue;
        }
        let end = find_slice(content, b"\r\n--").unwrap_or(content.len());
        return bounded(String::from_utf8_lossy(&content[..end]).trim().to_owned());
    }
    String::new()
}

fn is_csrf_part(headers: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(headers) else {
        return false;
    };
    let lower = text.to_ascii_lowercase();
    lower.contains("name=\"csrf_token\"") || lower.contains("name='csrf_token'")
}

fn bounded(token: String) -> String {
    if token.len() > 256 {
        String::new()
    } else {
        token
    }
}

fn decode(value: &str) -> String {
    let plus = value.replace('+', " ");
    let bytes = plus.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&plus[index + 1..index + 3], 16) {
                out.push(byte);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn find_slice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn prefers_html(headers: &HeaderMap) -> bool {
    let mime = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim();
    if mime.eq_ignore_ascii_case("application/x-www-form-urlencoded")
        || mime.eq_ignore_ascii_case("multipart/form-data")
    {
        return true;
    }
    if mime.eq_ignore_ascii_case("application/json") {
        return false;
    }
    headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("text/html") && !value.contains("application/json"))
}

fn reject(error: AuthError, headers: &HeaderMap) -> Response {
    let error = match error {
        AuthError::Unauthenticated => AuthError::CsrfRejected,
        other => other,
    };
    if prefers_html(headers) && matches!(error, AuthError::CsrfRejected) {
        return html_csrf(headers);
    }
    ApiError(FexuError::from(error)).into_response()
}

fn html_csrf(headers: &HeaderMap) -> Response {
    // Match the admin's locale priority: an explicit locale cookie wins.
    // Without a locale preference, new Fexu projects use English.
    let locale_cookie = cookie_value(headers, "fexu_locale");
    let locale = match locale_cookie.as_deref() {
        Some("de") => "de",
        Some("en") => "en",
        _ => {
            let accept_language = headers
                .get(header::ACCEPT_LANGUAGE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or("")
                .to_ascii_lowercase();
            if accept_language.contains("de") {
                "de"
            } else {
                "en"
            }
        }
    };
    let sentence = fexu_core::message_text(locale, "form.csrf");
    (
        StatusCode::FORBIDDEN,
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/html; charset=utf-8"),
        )],
        format!("<!DOCTYPE html><html><body><p>{sentence}</p></body></html>"),
    )
        .into_response()
}

fn problem(status: StatusCode, title: &'static str, detail: &'static str) -> Response {
    (status, Json(problem_json(status.as_u16(), title, detail))).into_response()
}
