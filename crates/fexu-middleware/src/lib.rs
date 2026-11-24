//! Standard Axum middleware. The most recently added layer sees requests first.
//!
//! Outer-to-inner order: body limit, request ID, access log, security headers, route.
//! Responses travel in reverse, so this layer sets the final security headers
//! after the handler returns.
//!
//! This layer owns `X-Content-Type-Options` and `X-Frame-Options`, always setting
//! them to `nosniff` and `DENY`. Applications cannot weaken these protections.
//! It also owns `Referrer-Policy`. All header fields and comma-separated tokens
//! are inspected. A single allowed policy value is retained as one canonical
//! header. Repeated identical allowed values are normalized to one header.
//! Conflicting, empty, or unsafe tokens such as `unsafe-url`, `origin`, and
//! `no-referrer-when-downgrade` are replaced with the configured policy.
//! Recovery pages retain their own `no-referrer` policy even when configuration
//! specifies `same-origin`. The default is `no-referrer`, customizable through
//! [`Standard::referrer_policy`]. This layer neither reads `Host`/`X-Forwarded-*`
//! nor performs Origin or CSRF validation.
//!
//! This crate provides neither authentication nor authorization. Cookie-based
//! CSRF protection is installed by `App::into_router`, not by this layer.

use axum::extract::{MatchedPath, Request};
use axum::http::{header, HeaderMap, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;
use axum::Router;
use std::time::Instant;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct RequestId(pub String);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReferrerPolicy {
    NoReferrer,
    SameOrigin,
    OriginWhenCrossOrigin,
    StrictOrigin,
    StrictOriginWhenCrossOrigin,
}

impl ReferrerPolicy {
    pub fn as_header(self) -> &'static str {
        match self {
            Self::NoReferrer => "no-referrer",
            Self::SameOrigin => "same-origin",
            Self::OriginWhenCrossOrigin => "origin-when-cross-origin",
            Self::StrictOrigin => "strict-origin",
            Self::StrictOriginWhenCrossOrigin => "strict-origin-when-cross-origin",
        }
    }

    /// Accepts only policies that this layer is permitted to set.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "no-referrer" => Some(Self::NoReferrer),
            "same-origin" => Some(Self::SameOrigin),
            "origin-when-cross-origin" => Some(Self::OriginWhenCrossOrigin),
            "strict-origin" => Some(Self::StrictOrigin),
            "strict-origin-when-cross-origin" => Some(Self::StrictOriginWhenCrossOrigin),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Standard {
    pub body_limit: usize,
    pub referrer_policy: ReferrerPolicy,
}

impl Default for Standard {
    fn default() -> Self {
        Self {
            body_limit: 2 * 1024 * 1024,
            referrer_policy: ReferrerPolicy::NoReferrer,
        }
    }
}

pub fn apply<S>(router: Router<S>, standard: Standard) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    let policy = standard.referrer_policy;
    router
        .layer(axum::middleware::from_fn(move |request, next| {
            let policy = policy;
            async move { security_headers(request, next, policy).await }
        }))
        .layer(axum::middleware::from_fn(access_log))
        .layer(axum::middleware::from_fn(request_id))
        .layer(axum::extract::DefaultBodyLimit::max(standard.body_limit))
}

async fn request_id(mut request: Request, next: Next) -> Response {
    let id = incoming_request_id(&request).unwrap_or_else(|| Uuid::new_v4().simple().to_string());
    request.extensions_mut().insert(RequestId(id.clone()));
    note(&mut request, "request_id");
    let mut response = next.run(request).await;
    if let Ok(value) = HeaderValue::from_str(&id) {
        response.headers_mut().insert("x-request-id", value);
    }
    response
}

async fn access_log(request: Request, next: Next) -> Response {
    let method = request.method().clone();
    let path = request
        .extensions()
        .get::<MatchedPath>()
        .map(|path| path.as_str())
        .unwrap_or("<unmatched>")
        .to_owned();
    let started = Instant::now();
    let id = request
        .extensions()
        .get::<RequestId>()
        .map(|value| value.0.clone())
        .unwrap_or_default();
    let mut request = request;
    note(&mut request, "access_log");
    let response = next.run(request).await;
    tracing::info!(
        method = %method,
        path = %path,
        status = response.status().as_u16(),
        elapsed_ms = started.elapsed().as_millis() as u64,
        request_id = %id,
        "http request"
    );
    response
}

async fn security_headers(mut request: Request, next: Next, policy: ReferrerPolicy) -> Response {
    note(&mut request, "security_headers");
    let order = request.extensions().get::<LayerOrder>().cloned();
    let mut response = next.run(request).await;
    if let Some(order) = order {
        if let Ok(value) = HeaderValue::from_str(&order.0.join(",")) {
            response.headers_mut().insert("x-fexu-middleware", value);
        }
    }
    let headers = response.headers_mut();
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    let policy = shared_referrer_policy(headers, policy);
    headers.remove(header::REFERRER_POLICY);
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static(policy.as_header()),
    );
    response
}

/// Ein erlaubter Policy-Wert, sonst die konfigurierte Policy.
///
/// Reads repeated header fields and comma-separated values individually.
/// The response retains the value only when every token names the same allowed policy.
fn shared_referrer_policy(headers: &HeaderMap, configured: ReferrerPolicy) -> ReferrerPolicy {
    let mut accepted = None;
    for value in headers.get_all(header::REFERRER_POLICY) {
        let Ok(text) = value.to_str() else {
            return configured;
        };
        for token in text.split(',') {
            let token = token.trim();
            if token.is_empty() {
                return configured;
            }
            let Some(policy) = ReferrerPolicy::parse(token) else {
                return configured;
            };
            if let Some(previous) = accepted {
                if previous != policy {
                    return configured;
                }
            } else {
                accepted = Some(policy);
            }
        }
    }
    accepted.unwrap_or(configured)
}

/// Returns the raw path without its query for callers that explicitly need it.
/// Standard access logging uses the matched route template instead, because
/// dynamic and unmatched paths may contain sensitive values.
pub fn logged_path(uri: &axum::http::Uri) -> String {
    uri.path().to_owned()
}

/// Accepts a caller-supplied request id when it is a short token.
///
/// Multiple header values or malformed tokens are ignored. A syntactically
/// valid inbound token is still untrusted caller data, not an identity.
pub fn incoming_request_id(request: &Request) -> Option<String> {
    let headers = request.headers().get_all("x-request-id");
    let mut values = headers.iter();
    let value = values.next()?.to_str().ok()?;
    if values.next().is_some() {
        return None;
    }
    if (8..=64).contains(&value.len())
        && value
            .chars()
            .all(|char| char.is_ascii_alphanumeric() || char == '-')
    {
        Some(value.to_owned())
    } else {
        None
    }
}

/// JSON lines when `FEXU_LOG_FORMAT=json`. The default remains the text subscriber.
///
/// `RUST_LOG` selects the filter. The default filter is `info`. This does not install OpenTelemetry.
pub fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    if structured_logs_requested() {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .json()
            .with_current_span(false)
            .with_span_list(false)
            .init();
    } else {
        tracing_subscriber::fmt().with_env_filter(filter).init();
    }
}

pub fn structured_logs_requested() -> bool {
    std::env::var("FEXU_LOG_FORMAT")
        .ok()
        .is_some_and(|value| value.eq_ignore_ascii_case("json"))
}

#[derive(Clone, Default)]
struct LayerOrder(Vec<String>);

fn note(request: &mut Request, name: &str) {
    if request.headers().get("x-fexu-trace-order").is_none() {
        return;
    }
    if let Some(order) = request.extensions_mut().get_mut::<LayerOrder>() {
        order.0.push(name.to_owned());
    } else {
        request
            .extensions_mut()
            .insert(LayerOrder(vec![name.to_owned()]));
    }
}

#[cfg(test)]
mod tests {
    use super::{apply, incoming_request_id, logged_path, ReferrerPolicy, Standard};
    use axum::extract::Request;
    use axum::response::IntoResponse;
    use axum::routing::get;
    use axum::Router;
    use std::io::{Read, Write};
    use std::net::TcpStream;

    #[test]
    fn query_secrets_stay_out_of_the_log_path() {
        let uri = "/login?password=secret&token=abc".parse().unwrap();
        assert_eq!(logged_path(&uri), "/login");
    }

    #[test]
    fn request_id_keeps_a_safe_inbound_token() {
        let mut request = Request::new(axum::body::Body::empty());
        request
            .headers_mut()
            .insert("x-request-id", "abc-1234".parse().unwrap());
        assert_eq!(incoming_request_id(&request).as_deref(), Some("abc-1234"));
        request
            .headers_mut()
            .append("x-request-id", "second-1234".parse().unwrap());
        assert!(incoming_request_id(&request).is_none());
        request
            .headers_mut()
            .insert("x-request-id", "short".parse().unwrap());
        assert!(incoming_request_id(&request).is_none());
        request
            .headers_mut()
            .insert("x-request-id", "has space".parse().unwrap());
        assert!(incoming_request_id(&request).is_none());
    }

    #[test]
    fn request_id_and_security_headers_are_set() {
        let app = apply(
            Router::new().route("/health", get(|| async { "ok" })),
            Standard::default(),
        );
        let body = exchange(app, "GET /health?password=secret HTTP/1.1\r\nHost: localhost\r\nCookie: fexu_session=secret\r\nConnection: close\r\n\r\n");
        let lower = body.to_ascii_lowercase();
        assert!(lower.contains("x-request-id:"), "{body}");
        assert!(lower.contains("x-content-type-options: nosniff"));
        assert!(lower.contains("x-frame-options: deny"));
        assert!(lower.contains("referrer-policy: no-referrer"));
        assert!(!lower.contains("password=secret"));
        assert!(!lower.contains("fexu_session=secret"));
    }

    #[test]
    fn explicit_referrer_policy_survives_the_full_chain() {
        let app = apply(
            Router::new().route(
                "/health",
                get(|| async { ([(axum::http::header::REFERRER_POLICY, "same-origin")], "ok") }),
            ),
            Standard {
                referrer_policy: super::ReferrerPolicy::NoReferrer,
                ..Standard::default()
            },
        );
        let body = exchange(
            app,
            "GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        );
        let lower = body.to_ascii_lowercase();
        assert!(lower.contains("referrer-policy: same-origin"), "{body}");
        assert!(!lower.contains("referrer-policy: no-referrer"), "{body}");
        assert!(lower.contains("x-frame-options: deny"), "{body}");
        assert!(lower.contains("x-content-type-options: nosniff"), "{body}");
    }

    #[test]
    fn configured_referrer_policy_fills_a_missing_header() {
        let app = apply(
            Router::new().route("/health", get(|| async { "ok" })),
            Standard {
                referrer_policy: super::ReferrerPolicy::SameOrigin,
                ..Standard::default()
            },
        );
        let body = exchange(
            app,
            "GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        );
        assert!(
            body.to_ascii_lowercase()
                .contains("referrer-policy: same-origin"),
            "{body}"
        );
    }

    #[test]
    fn unsafe_referrer_policy_is_replaced_without_weakening_other_headers() {
        for unsafe_policy in [
            "unsafe-url",
            "origin",
            "no-referrer-when-downgrade",
            "",
            "same-origin, no-referrer",
        ] {
            let app = apply(
                Router::new().route(
                    "/health",
                    get(move || async move {
                        ([(axum::http::header::REFERRER_POLICY, unsafe_policy)], "ok")
                    }),
                ),
                Standard::default(),
            );
            let body = exchange(
                app,
                "GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
            );
            let lower = body.to_ascii_lowercase();
            assert!(
                lower.contains("referrer-policy: no-referrer"),
                "{unsafe_policy} -> {body}"
            );
            assert!(!lower.contains("unsafe-url"), "{body}");
            assert!(!lower.contains("no-referrer-when-downgrade"), "{body}");
            assert!(lower.contains("x-frame-options: deny"), "{body}");
            assert!(lower.contains("x-content-type-options: nosniff"), "{body}");
        }
    }

    #[test]
    fn referrer_policy_collapses_to_one_header_after_the_full_chain() {
        let cases = [
            (
                ReferrerPolicy::NoReferrer,
                &["same-origin", "unsafe-url"][..],
                "no-referrer",
            ),
            (
                ReferrerPolicy::NoReferrer,
                &["unsafe-url", "same-origin"][..],
                "no-referrer",
            ),
            (
                ReferrerPolicy::NoReferrer,
                &["same-origin, unsafe-url"][..],
                "no-referrer",
            ),
            (
                ReferrerPolicy::NoReferrer,
                &["same-origin", "no-referrer"][..],
                "no-referrer",
            ),
            (
                ReferrerPolicy::SameOrigin,
                &["no-referrer", "strict-origin"][..],
                "same-origin",
            ),
            (
                ReferrerPolicy::NoReferrer,
                &["same-origin", "same-origin"][..],
                "same-origin",
            ),
            (
                ReferrerPolicy::NoReferrer,
                &["same-origin, same-origin"][..],
                "same-origin",
            ),
            (
                ReferrerPolicy::NoReferrer,
                &["SAME-ORIGIN", "same-origin"][..],
                "same-origin",
            ),
            (
                ReferrerPolicy::NoReferrer,
                &["same-origin"][..],
                "same-origin",
            ),
            (ReferrerPolicy::NoReferrer, &[][..], "no-referrer"),
            (ReferrerPolicy::SameOrigin, &[][..], "same-origin"),
        ];
        for (configured, values, expected) in cases {
            let body = exchange(
                routed(values, configured),
                "GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
            );
            let head = body.split("\r\n\r\n").next().unwrap_or(&body);
            let found: Vec<_> = head
                .lines()
                .filter_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("referrer-policy")
                        .then(|| value.trim().to_owned())
                })
                .collect();
            assert_eq!(found, vec![expected.to_owned()], "{values:?} -> {body}");
            let lower = body.to_ascii_lowercase();
            assert!(!lower.contains("unsafe-url"), "{body}");
            assert!(lower.contains("x-frame-options: deny"), "{body}");
            assert!(lower.contains("x-content-type-options: nosniff"), "{body}");
        }
    }

    fn routed(values: &'static [&'static str], policy: ReferrerPolicy) -> Router {
        apply(
            Router::new().route(
                "/health",
                get(move || async move {
                    let mut response = "ok".into_response();
                    for value in values {
                        response.headers_mut().append(
                            axum::http::header::REFERRER_POLICY,
                            axum::http::HeaderValue::from_static(value),
                        );
                    }
                    response
                }),
            ),
            Standard {
                referrer_policy: policy,
                ..Standard::default()
            },
        )
    }

    #[test]
    fn recovery_no_referrer_survives_a_same_origin_default() {
        let app = apply(
            Router::new().route(
                "/auth/password-reset",
                get(|| async { ([(axum::http::header::REFERRER_POLICY, "no-referrer")], "ok") }),
            ),
            Standard {
                referrer_policy: super::ReferrerPolicy::SameOrigin,
                ..Standard::default()
            },
        );
        let body = exchange(
            app,
            "GET /auth/password-reset HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        );
        let lower = body.to_ascii_lowercase();
        assert!(lower.contains("referrer-policy: no-referrer"), "{body}");
        assert!(!lower.contains("referrer-policy: same-origin"), "{body}");
    }

    #[test]
    fn frame_and_content_type_headers_are_not_weakened() {
        let app = apply(
            Router::new().route(
                "/health",
                get(|| async {
                    (
                        [
                            (axum::http::header::X_FRAME_OPTIONS, "ALLOWALL"),
                            (axum::http::header::X_CONTENT_TYPE_OPTIONS, "allow"),
                        ],
                        "ok",
                    )
                }),
            ),
            Standard::default(),
        );
        let body = exchange(
            app,
            "GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        );
        let lower = body.to_ascii_lowercase();
        assert!(lower.contains("x-frame-options: deny"), "{body}");
        assert!(!lower.contains("allowall"), "{body}");
        assert!(lower.contains("x-content-type-options: nosniff"), "{body}");
    }

    #[test]
    fn oversized_body_is_rejected() {
        let app = apply(
            Router::new().route(
                "/echo",
                axum::routing::post(|_body: axum::body::Bytes| async { "ok" }),
            ),
            Standard {
                body_limit: 8,
                ..Standard::default()
            },
        );
        let body = exchange(app, "POST /echo HTTP/1.1\r\nHost: localhost\r\nContent-Length: 20\r\nConnection: close\r\n\r\n01234567890123456789");
        assert!(body.contains("413"), "{body}");
    }

    #[test]
    fn middleware_runs_request_id_before_logging_and_headers() {
        let app = apply(
            Router::new().route("/health", get(|| async { "ok" })),
            Standard::default(),
        );
        let body = exchange(app, "GET /health HTTP/1.1\r\nHost: localhost\r\nx-fexu-trace-order: 1\r\nConnection: close\r\n\r\n");
        let lower = body.to_ascii_lowercase();
        assert!(
            lower.contains("x-fexu-middleware: request_id,access_log,security_headers"),
            "{body}"
        );
    }

    fn exchange(app: Router, request: &str) -> String {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let listener = runtime
            .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
            .unwrap();
        let address = listener.local_addr().unwrap();
        runtime.spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let mut stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        let mut body = String::new();
        let _ = stream.read_to_string(&mut body);
        body
    }
}
