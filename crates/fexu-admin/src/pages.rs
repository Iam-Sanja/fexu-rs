//! Eigene Admin-Seiten unter `/admin/pages/{key}`.

use crate::html::{self, error_page, locale_of, parse_form, require_get, require_post};
use crate::registry::AdminRegistry;
use crate::views::{internal_admin_location, UploadedFile, ViewOutcome, ViewQuery, ViewRequest};
use axum::body::Bytes;
use axum::extract::{Extension, OriginalUri, Path, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use axum::response::Response;
use fexu_auth::AppState;
use std::collections::HashMap;

pub async fn show(
    State(state): State<AppState>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    Extension(registry): Extension<AdminRegistry>,
    Path(key): Path<String>,
) -> Response {
    render(&state, &headers, &registry, &key, uri.query(), None).await
}

pub async fn submit(
    State(state): State<AppState>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    Extension(registry): Extension<AdminRegistry>,
    Path(key): Path<String>,
    body: Bytes,
) -> Response {
    render(&state, &headers, &registry, &key, uri.query(), Some(body)).await
}

async fn render(
    state: &AppState,
    headers: &HeaderMap,
    registry: &AdminRegistry,
    key: &str,
    raw_query: Option<&str>,
    body: Option<Bytes>,
) -> Response {
    let locale = locale_of(
        headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    if !registry.sealed() {
        return error_page(
            state,
            headers,
            registry,
            &locale,
            StatusCode::INTERNAL_SERVER_ERROR,
            "forbidden",
            false,
            "",
        )
        .await;
    }
    let Some(view) = registry.view_by_key(key) else {
        let signed = require_get(state, headers, registry, "/admin").await.ok();
        return error_page(
            state,
            headers,
            registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            signed.is_some(),
            signed
                .as_ref()
                .map(|user| user.email.as_str())
                .unwrap_or(""),
        )
        .await;
    };
    if let Some(bytes) = body.as_ref() {
        if multipart_type(headers).is_some() && bytes.len() > fexu_storage::MULTIPART_BODY_LIMIT {
            return error_page(
                state,
                headers,
                registry,
                &locale,
                StatusCode::PAYLOAD_TOO_LARGE,
                "validation",
                false,
                "",
            )
            .await;
        }
    }
    let user = if let Some(bytes) = body.as_ref() {
        match require_post(state, headers, bytes).await {
            Ok(user) => user,
            Err(()) => {
                return error_page(
                    state,
                    headers,
                    registry,
                    &locale,
                    StatusCode::FORBIDDEN,
                    "csrf",
                    true,
                    "",
                )
                .await;
            }
        }
    } else {
        match require_get(state, headers, registry, &format!("/admin/pages/{key}")).await {
            Ok(user) => user,
            Err(response) => return response,
        }
    };
    let allowed = match holds(state, &user, view.permission_code()).await {
        Ok(true) => true,
        Ok(false) => false,
        Err(()) => {
            return error_page(
                state,
                headers,
                registry,
                &locale,
                StatusCode::INTERNAL_SERVER_ERROR,
                "forbidden",
                true,
                &user.email,
            )
            .await;
        }
    };
    if !allowed {
        return error_page(
            state,
            headers,
            registry,
            &locale,
            StatusCode::FORBIDDEN,
            "forbidden",
            true,
            &user.email,
        )
        .await;
    }
    let handler = if body.is_some() {
        view.post_handler()
    } else {
        view.get_handler()
    };
    let Some(handler) = handler else {
        return error_page(
            state,
            headers,
            registry,
            &locale,
            StatusCode::METHOD_NOT_ALLOWED,
            "forbidden",
            true,
            &user.email,
        )
        .await;
    };
    let parsed = match body.as_ref() {
        Some(bytes) => match multipart_type(headers) {
            Some(content_type) => match fexu_storage::read_multipart(content_type, bytes) {
                Ok(parts) => match split_upload(parts) {
                    Ok(parsed) => parsed,
                    Err(()) => {
                        return error_page(
                            state,
                            headers,
                            registry,
                            &locale,
                            StatusCode::BAD_REQUEST,
                            "validation",
                            true,
                            &user.email,
                        )
                        .await;
                    }
                },
                Err(_) => {
                    return error_page(
                        state,
                        headers,
                        registry,
                        &locale,
                        StatusCode::BAD_REQUEST,
                        "validation",
                        true,
                        &user.email,
                    )
                    .await;
                }
            },
            None => (parse_form(bytes), Vec::new()),
        },
        None => (HashMap::new(), Vec::new()),
    };
    let (form, uploads) = parsed;
    let query = match ViewQuery::parse(raw_query) {
        Ok(query) => query,
        Err(error) => {
            tracing::warn!(code = error.code, "custom admin query rejected");
            return error_page(
                state,
                headers,
                registry,
                &locale,
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_filter",
                true,
                &user.email,
            )
            .await;
        }
    };
    let request = ViewRequest {
        user: &user,
        db: &state.db,
        locale: &locale,
        query: &query,
        form: &form,
        uploads: &uploads,
    };
    let outcome = match handler.handle(&request).await {
        Ok(outcome) => outcome,
        Err(error) => {
            let (status, key) = match error.code {
                "extension.query" => (StatusCode::UNPROCESSABLE_ENTITY, "invalid_filter"),
                "extension.redirect" => (StatusCode::BAD_REQUEST, "redirect_rejected"),
                _ => (StatusCode::INTERNAL_SERVER_ERROR, "forbidden"),
            };
            if status == StatusCode::INTERNAL_SERVER_ERROR {
                tracing::error!(
                    code = error.code,
                    detail = %error.detail,
                    "custom admin view failed"
                );
            } else {
                tracing::warn!(code = error.code, "custom admin view rejected");
            }
            return error_page(
                state,
                headers,
                registry,
                &locale,
                status,
                key,
                true,
                &user.email,
            )
            .await;
        }
    };
    match outcome {
        ViewOutcome::Page(rendered) => {
            let csrf = html::session_csrf(state.auth.as_ref().unwrap(), headers).await;
            let data = serde_json::Value::Object(rendered.data);
            html::page_as(
                state,
                &user,
                registry,
                view.template_name(),
                &locale,
                html::theme_of(headers),
                view.title(&locale),
                true,
                user.is_superuser,
                &user.email,
                &csrf,
                &format!("/admin/pages/{key}"),
                &clip(&rendered.notice),
                &clip(&rendered.alert),
                data,
            )
            .await
        }
        ViewOutcome::Redirect(location) => match internal_admin_location(&location) {
            Some(safe) if safe == location => see_other(&safe),
            _ => {
                error_page(
                    state,
                    headers,
                    registry,
                    &locale,
                    StatusCode::BAD_REQUEST,
                    "redirect_rejected",
                    true,
                    &user.email,
                )
                .await
            }
        },
        ViewOutcome::NotFound => {
            error_page(
                state,
                headers,
                registry,
                &locale,
                StatusCode::NOT_FOUND,
                "not_found",
                true,
                &user.email,
            )
            .await
        }
        ViewOutcome::Conflict => {
            error_page(
                state,
                headers,
                registry,
                &locale,
                StatusCode::CONFLICT,
                "conflict",
                true,
                &user.email,
            )
            .await
        }
        ViewOutcome::Invalid => {
            error_page(
                state,
                headers,
                registry,
                &locale,
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_filter",
                true,
                &user.email,
            )
            .await
        }
    }
}

fn see_other(location: &str) -> Response {
    let mut response = StatusCode::SEE_OTHER.into_response();
    let Ok(value) = HeaderValue::from_str(location) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    response.headers_mut().insert(header::LOCATION, value);
    response
}

fn clip(value: &str) -> String {
    value.chars().take(200).collect()
}

fn multipart_type(headers: &HeaderMap) -> Option<&str> {
    let value = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())?;
    value
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .eq_ignore_ascii_case("multipart/form-data")
        .then_some(value)
}

fn split_upload(
    parts: Vec<fexu_storage::FormPart>,
) -> Result<(HashMap<String, String>, Vec<UploadedFile>), ()> {
    let mut form = HashMap::new();
    let mut uploads = Vec::new();
    for part in parts {
        if let Some(filename) = part.filename {
            uploads.push(UploadedFile {
                field: part.name,
                filename,
                declared_type: part.declared_type,
                bytes: part.bytes,
            });
        } else {
            let text = String::from_utf8(part.bytes).map_err(|_| ())?;
            form.insert(part.name, text);
        }
    }
    Ok((form, uploads))
}

async fn holds(state: &AppState, user: &fexu_auth::AuthUser, code: &str) -> Result<bool, ()> {
    let Some(auth) = state.auth.as_ref() else {
        return Err(());
    };
    auth.holds(user, code).await.map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::{show, submit};
    use crate::registry::AdminRegistry;
    use crate::views::{
        AdminView, ExtensionError, ViewHandler, ViewOutcome, ViewPage, ViewRequest,
    };
    use axum::body::Bytes;
    use axum::extract::{Extension, OriginalUri, Path, State};
    use axum::http::{HeaderMap, StatusCode};
    use fexu_auth::{AppState, AuthConfig, AuthService};
    use std::sync::{Arc, Mutex};

    fn memory_fixture_password() -> String {
        String::from_utf8(vec![
            99, 111, 114, 114, 101, 99, 116, 45, 104, 111, 114, 115, 101, 45, 98, 97, 116, 116,
            101, 114, 121,
        ])
        .expect("fixture password")
    }

    struct Capture(Arc<Mutex<Option<(String, String, usize)>>>);

    #[async_trait::async_trait]
    impl ViewHandler for Capture {
        async fn handle(&self, request: &ViewRequest<'_>) -> Result<ViewOutcome, ExtensionError> {
            let file = request.uploads.first();
            *self.0.lock().expect("capture") = Some((
                request.form.get("title").cloned().unwrap_or_default(),
                file.map(|file| file.filename.clone()).unwrap_or_default(),
                file.map(|file| file.size()).unwrap_or(0),
            ));
            Ok(ViewPage::new(Default::default()).into())
        }
    }

    async fn service() -> AuthService {
        let db = fexu_db::connect("sqlite::memory:").await.unwrap();
        fexu_auth::apply_migrations(&db).await.unwrap();
        AuthService::new(db, AuthConfig::development())
    }

    fn registry(capture: Arc<Mutex<Option<(String, String, usize)>>>) -> AdminRegistry {
        let handler = Capture(capture);
        AdminRegistry::new()
            .view(
                AdminView::new("uploads")
                    .titles("Uploads", "Uploads")
                    .permission("catalog.change")
                    .on_get(Capture(handler.0.clone()))
                    .on_post(handler),
            )
            .unwrap()
            .seal()
            .unwrap()
    }

    fn cookie_pair(set_cookie: &str) -> String {
        set_cookie.split(';').next().unwrap().to_owned()
    }

    fn multipart(parts: &str) -> Bytes {
        Bytes::from(format!("{parts}--bound--\r\n"))
    }

    async fn post(
        auth: &AuthService,
        registry: AdminRegistry,
        headers: HeaderMap,
        body: Bytes,
    ) -> StatusCode {
        let state = AppState {
            db: auth.database().clone(),
            auth: Some(auth.clone()),
            mail: None,
        };
        let response = submit(
            State(state),
            headers,
            OriginalUri(axum::http::Uri::from_static("/admin/pages/uploads")),
            Extension(registry),
            Path("uploads".to_owned()),
            body,
        )
        .await;
        response.status()
    }

    #[tokio::test]
    async fn custom_page_accepts_multipart_and_keeps_urlencoded_forms() {
        let auth = service().await;
        let password = memory_fixture_password();
        let root = auth
            .create_user("root@example.com", &password, true, true)
            .await
            .unwrap();
        let issued = auth
            .login("root@example.com", &password, "peer")
            .await
            .unwrap();
        let _ = root;
        let capture = Arc::new(Mutex::new(None));
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::COOKIE,
            cookie_pair(&issued.cookie).parse().unwrap(),
        );
        headers.insert(
            axum::http::header::CONTENT_TYPE,
            "multipart/form-data; boundary=bound".parse().unwrap(),
        );
        let body = multipart(&format!(
            "--bound\r\nContent-Disposition: form-data; name=\"csrf_token\"\r\n\r\n{}\r\n\
--bound\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\nHammer\r\n\
--bound\r\nContent-Disposition: form-data; name=\"file\"; filename=\"photo.png\"\r\nContent-Type: image/png\r\n\r\nPNG\r\n\
--bound\r\nContent-Disposition: form-data; name=\"extra\"; filename=\"photo.jpg\"\r\nContent-Type: image/jpeg\r\n\r\nJPG\r\n",
            issued.csrf_token
        ));
        let status = post(&auth, registry(capture.clone()), headers.clone(), body).await;
        assert_eq!(status, StatusCode::OK);
        let seen = capture.lock().unwrap().clone().unwrap();
        assert_eq!(seen.0, "Hammer");
        assert_eq!(seen.1, "photo.png");
        assert_eq!(seen.2, 3);

        *capture.lock().unwrap() = None;
        let mut form_headers = HeaderMap::new();
        form_headers.insert(
            axum::http::header::COOKIE,
            cookie_pair(&issued.cookie).parse().unwrap(),
        );
        form_headers.insert(
            axum::http::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded".parse().unwrap(),
        );
        let status = post(
            &auth,
            registry(capture.clone()),
            form_headers,
            Bytes::from(format!("csrf_token={}&title=Plain", issued.csrf_token)),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let seen = capture.lock().unwrap().clone().unwrap();
        assert_eq!(seen.0, "Plain");
        assert!(seen.1.is_empty());

        *capture.lock().unwrap() = None;
        let status = post(
            &auth,
            registry(capture.clone()),
            headers.clone(),
            multipart(
                "--bound\r\nContent-Disposition: form-data; name=\"file\"; filename=\"../x.png\"\r\n\r\nA\r\n",
            ),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(capture.lock().unwrap().is_none());

        let bad = headers.clone();
        let status = post(
            &auth,
            registry(capture.clone()),
            bad,
            multipart(&format!(
                "--bound\r\nContent-Disposition: form-data; name=\"csrf_token\"\r\n\r\n{}\r\n\
--bound\r\nContent-Disposition: form-data; name=\"file\"; filename=\"../x.png\"\r\n\r\nA\r\n",
                issued.csrf_token
            )),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(capture.lock().unwrap().is_none());

        let status = post(
            &auth,
            registry(capture.clone()),
            headers,
            multipart(
                "--bound\r\nContent-Disposition: form-data; name=\"csrf_token\"\r\n\r\nwrong\r\n\
--bound\r\nContent-Disposition: form-data; name=\"file\"; filename=\"photo.png\"\r\n\r\nPNG\r\n",
            ),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(capture.lock().unwrap().is_none());

        let anonymous = post(
            &auth,
            registry(capture.clone()),
            HeaderMap::new(),
            multipart(
                "--bound\r\nContent-Disposition: form-data; name=\"file\"; filename=\"photo.png\"\r\n\r\nPNG\r\n",
            ),
        )
        .await;
        assert_eq!(anonymous, StatusCode::FORBIDDEN);
        assert!(capture.lock().unwrap().is_none());

        let clerk = auth
            .create_user("clerk@example.com", &password, true, false)
            .await
            .unwrap();
        let clerk_session = auth
            .login("clerk@example.com", &password, "peer")
            .await
            .unwrap();
        let _ = clerk;
        let mut clerk_headers = HeaderMap::new();
        clerk_headers.insert(
            axum::http::header::COOKIE,
            cookie_pair(&clerk_session.cookie).parse().unwrap(),
        );
        clerk_headers.insert(
            axum::http::header::CONTENT_TYPE,
            "multipart/form-data; boundary=bound".parse().unwrap(),
        );
        let status = post(
            &auth,
            registry(capture.clone()),
            clerk_headers,
            multipart(&format!(
                "--bound\r\nContent-Disposition: form-data; name=\"csrf_token\"\r\n\r\n{}\r\n\
--bound\r\nContent-Disposition: form-data; name=\"file\"; filename=\"photo.png\"\r\n\r\nPNG\r\n",
                clerk_session.csrf_token
            )),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(capture.lock().unwrap().is_none());

        let mut huge = HeaderMap::new();
        huge.insert(
            axum::http::header::CONTENT_TYPE,
            "multipart/form-data; boundary=bound".parse().unwrap(),
        );
        let status = post(
            &auth,
            registry(capture.clone()),
            huge,
            Bytes::from(vec![b'x'; fexu_storage::MULTIPART_BODY_LIMIT + 1]),
        )
        .await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
        assert!(capture.lock().unwrap().is_none());
    }

    #[derive(Clone, Copy)]
    enum Mode {
        Page,
        Redirect,
        External,
        OffMount,
        Missing,
        Conflict,
        Invalid,
    }

    struct FilterView {
        calls: Arc<Mutex<u32>>,
        reads: Arc<Mutex<u32>>,
        mode: Arc<Mutex<Mode>>,
    }

    #[async_trait::async_trait]
    impl ViewHandler for FilterView {
        async fn handle(&self, request: &ViewRequest<'_>) -> Result<ViewOutcome, ExtensionError> {
            *self.calls.lock().expect("calls") += 1;
            match *self.mode.lock().expect("mode") {
                Mode::Page => {
                    let date = request.query.date("date")?;
                    *self.reads.lock().expect("reads") += 1;
                    let mut data = serde_json::Map::new();
                    data.insert(
                        "body".to_owned(),
                        serde_json::json!("<script>alert(1)</script>"),
                    );
                    data.insert(
                        "date".to_owned(),
                        serde_json::json!(date.map(|value| value.to_string()).unwrap_or_default()),
                    );
                    let mut page = ViewPage::new(data);
                    page.notice = "<b>ok</b>".to_owned();
                    Ok(page.into())
                }
                Mode::Redirect => ViewOutcome::redirect("/admin/pages/reports?date=2026-01-02"),
                Mode::External => ViewOutcome::redirect("https://evil.example/admin"),
                Mode::OffMount => ViewOutcome::redirect("/api/secret"),
                Mode::Missing => Ok(ViewOutcome::not_found()),
                Mode::Conflict => Ok(ViewOutcome::conflict()),
                Mode::Invalid => Ok(ViewOutcome::invalid()),
            }
        }
    }

    fn filter_registry(
        calls: Arc<Mutex<u32>>,
        reads: Arc<Mutex<u32>>,
        mode: Arc<Mutex<Mode>>,
    ) -> AdminRegistry {
        let handler = FilterView {
            calls: calls.clone(),
            reads: reads.clone(),
            mode: mode.clone(),
        };
        AdminRegistry::new()
            .view(
                AdminView::new("reports")
                    .titles("Reports", "Reports")
                    .permission("reports.read")
                    .on_get(FilterView { calls, reads, mode })
                    .on_post(handler),
            )
            .unwrap()
            .seal()
            .unwrap()
    }

    async fn text(response: axum::response::Response) -> String {
        use http_body_util::BodyExt;
        String::from_utf8(
            response
                .into_body()
                .collect()
                .await
                .expect("body")
                .to_bytes()
                .to_vec(),
        )
        .expect("utf-8")
    }

    fn location(response: &axum::response::Response) -> Option<String> {
        response
            .headers()
            .get(axum::http::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    }

    #[tokio::test]
    async fn custom_page_reads_filters_and_rejects_open_redirects() {
        let auth = service().await;
        let password = memory_fixture_password();
        let root = auth
            .create_user("root@example.com", &password, true, true)
            .await
            .unwrap();
        let issued = auth
            .login("root@example.com", &password, "peer")
            .await
            .unwrap();
        let _ = root;
        let calls = Arc::new(Mutex::new(0));
        let reads = Arc::new(Mutex::new(0));
        let mode = Arc::new(Mutex::new(Mode::Page));
        let registry = filter_registry(calls.clone(), reads.clone(), mode.clone());
        let state = AppState {
            db: auth.database().clone(),
            auth: Some(auth.clone()),
            mail: None,
        };
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::COOKIE,
            cookie_pair(&issued.cookie).parse().unwrap(),
        );

        let page = show(
            State(state.clone()),
            headers.clone(),
            OriginalUri("/admin/pages/reports?date=2026-01-02".parse().unwrap()),
            Extension(registry.clone()),
            Path("reports".to_owned()),
        )
        .await;
        assert_eq!(page.status(), StatusCode::OK);
        assert!(location(&page).is_none());
        let body = text(page).await;
        assert!(body.contains("class=\"filter\">2026-01-02"), "{body}");
        assert!(
            body.contains("&lt;script&gt;alert(1)&lt;&#x2F;script&gt;"),
            "{body}"
        );
        assert!(body.contains("&lt;b&gt;ok&lt;&#x2F;b&gt;"), "{body}");
        assert!(!body.contains("<script>alert(1)</script>"), "{body}");
        assert!(!body.contains("<b>ok</b>"), "{body}");
        assert!(
            body.contains("value=\"&#x2F;admin&#x2F;pages&#x2F;reports\""),
            "{body}"
        );
        assert_eq!(*calls.lock().unwrap(), 1);
        assert_eq!(*reads.lock().unwrap(), 1);

        *calls.lock().unwrap() = 0;
        *reads.lock().unwrap() = 0;
        let invalid = show(
            State(state.clone()),
            headers.clone(),
            OriginalUri("/admin/pages/reports?date=2026-99-99".parse().unwrap()),
            Extension(registry.clone()),
            Path("reports".to_owned()),
        )
        .await;
        assert_eq!(invalid.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(location(&invalid).is_none());
        let invalid_body = text(invalid).await;
        assert!(
            invalid_body.contains("filter is invalid")
                || invalid_body.contains("Filter ist ungültig"),
            "{invalid_body}"
        );
        assert_eq!(*calls.lock().unwrap(), 1);
        assert_eq!(*reads.lock().unwrap(), 0);

        *calls.lock().unwrap() = 0;
        let hostile = show(
            State(state.clone()),
            headers.clone(),
            OriginalUri("/admin/pages/reports?date=%3Cscript%3E".parse().unwrap()),
            Extension(registry.clone()),
            Path("reports".to_owned()),
        )
        .await;
        assert_eq!(hostile.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(*calls.lock().unwrap(), 0);
        assert!(!text(hostile).await.contains("<script>"));

        *mode.lock().unwrap() = Mode::Redirect;
        headers.insert(
            axum::http::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded".parse().unwrap(),
        );
        let redirected = submit(
            State(state.clone()),
            headers.clone(),
            OriginalUri("/admin/pages/reports".parse().unwrap()),
            Extension(registry.clone()),
            Path("reports".to_owned()),
            Bytes::from(format!("csrf_token={}", issued.csrf_token)),
        )
        .await;
        assert_eq!(redirected.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            location(&redirected).as_deref(),
            Some("/admin/pages/reports?date=2026-01-02")
        );
        assert!(!location(&redirected).unwrap().contains("evil"));

        *mode.lock().unwrap() = Mode::External;
        let external = submit(
            State(state.clone()),
            headers.clone(),
            OriginalUri("/admin/pages/reports".parse().unwrap()),
            Extension(registry.clone()),
            Path("reports".to_owned()),
            Bytes::from(format!("csrf_token={}", issued.csrf_token)),
        )
        .await;
        assert_eq!(external.status(), StatusCode::BAD_REQUEST);
        assert!(location(&external).is_none());
        let external_body = text(external).await;
        assert!(
            external_body.contains("redirect was rejected")
                || external_body.contains("Weiterleitung wurde abgelehnt"),
            "{external_body}"
        );
        assert!(!external_body.contains("evil.example"), "{external_body}");

        *mode.lock().unwrap() = Mode::OffMount;
        let off_mount = submit(
            State(state.clone()),
            headers.clone(),
            OriginalUri("/admin/pages/reports".parse().unwrap()),
            Extension(registry.clone()),
            Path("reports".to_owned()),
            Bytes::from(format!("csrf_token={}", issued.csrf_token)),
        )
        .await;
        assert_eq!(off_mount.status(), StatusCode::BAD_REQUEST);
        assert!(location(&off_mount).is_none());

        *mode.lock().unwrap() = Mode::Missing;
        let missing = show(
            State(state.clone()),
            headers.clone(),
            OriginalUri("/admin/pages/reports".parse().unwrap()),
            Extension(registry.clone()),
            Path("reports".to_owned()),
        )
        .await;
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);

        *mode.lock().unwrap() = Mode::Conflict;
        let conflict = show(
            State(state.clone()),
            headers.clone(),
            OriginalUri("/admin/pages/reports".parse().unwrap()),
            Extension(registry.clone()),
            Path("reports".to_owned()),
        )
        .await;
        assert_eq!(conflict.status(), StatusCode::CONFLICT);
        let conflict_body = text(conflict).await;
        assert!(
            conflict_body.contains("conflicts with the current state")
                || conflict_body.contains("widerspricht"),
            "{conflict_body}"
        );

        *mode.lock().unwrap() = Mode::Invalid;
        let rejected = show(
            State(state.clone()),
            headers.clone(),
            OriginalUri("/admin/pages/reports".parse().unwrap()),
            Extension(registry.clone()),
            Path("reports".to_owned()),
        )
        .await;
        assert_eq!(rejected.status(), StatusCode::UNPROCESSABLE_ENTITY);

        let calls_before = *calls.lock().unwrap();
        let forged = submit(
            State(state.clone()),
            headers.clone(),
            OriginalUri(
                "/admin/pages/reports?next=https://evil.example"
                    .parse()
                    .unwrap(),
            ),
            Extension(registry.clone()),
            Path("reports".to_owned()),
            Bytes::from("csrf_token=forged"),
        )
        .await;
        assert_eq!(forged.status(), StatusCode::FORBIDDEN);
        assert!(location(&forged).is_none());
        assert_eq!(*calls.lock().unwrap(), calls_before);

        let anonymous = show(
            State(state.clone()),
            HeaderMap::new(),
            OriginalUri(
                "/admin/pages/reports?next=https://evil.example"
                    .parse()
                    .unwrap(),
            ),
            Extension(registry.clone()),
            Path("reports".to_owned()),
        )
        .await;
        assert_eq!(anonymous.status(), StatusCode::SEE_OTHER);
        let login = location(&anonymous).unwrap();
        assert_eq!(login, "/admin/login?next=/admin/pages/reports");
        assert!(!login.contains("evil"));
        assert_eq!(*calls.lock().unwrap(), calls_before);

        let visitor = auth
            .create_user("visitor@example.com", &password, false, false)
            .await
            .unwrap();
        let visitor_session = auth
            .login("visitor@example.com", &password, "peer")
            .await
            .unwrap();
        let _ = visitor;
        let mut visitor_headers = HeaderMap::new();
        visitor_headers.insert(
            axum::http::header::COOKIE,
            cookie_pair(&visitor_session.cookie).parse().unwrap(),
        );
        let denied_staff = show(
            State(state.clone()),
            visitor_headers,
            OriginalUri("/admin/pages/reports?date=2026-01-02".parse().unwrap()),
            Extension(registry.clone()),
            Path("reports".to_owned()),
        )
        .await;
        assert_eq!(denied_staff.status(), StatusCode::FORBIDDEN);
        assert_eq!(*calls.lock().unwrap(), calls_before);

        let clerk = auth
            .create_user("clerk@example.com", &password, true, false)
            .await
            .unwrap();
        let clerk_session = auth
            .login("clerk@example.com", &password, "peer")
            .await
            .unwrap();
        let _ = clerk;
        let mut clerk_headers = HeaderMap::new();
        clerk_headers.insert(
            axum::http::header::COOKIE,
            cookie_pair(&clerk_session.cookie).parse().unwrap(),
        );
        let denied = show(
            State(state),
            clerk_headers,
            OriginalUri("/admin/pages/reports?date=2026-01-02".parse().unwrap()),
            Extension(registry),
            Path("reports".to_owned()),
        )
        .await;
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);
        assert_eq!(*calls.lock().unwrap(), calls_before);
        assert_eq!(*reads.lock().unwrap(), 0);
    }
}
