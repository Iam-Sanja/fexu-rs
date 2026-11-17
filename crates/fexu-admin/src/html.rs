//! Serverseitige Admin-Seiten. Schreibende Aktionen sind POST und nutzen das bestehende CSRF.

use crate::actions::ActionOperation;
use crate::audit::{self, Event};
use crate::configure::ManyLink;
use crate::drafts;
use crate::http::{
    access_scope, display_column, filters_from, gate, list_blocked, permit, reject_locked,
};
use crate::localization::{bulk_notice, catalog, normalize, notice, validation_text};
use crate::registry::{redact, sensitive, AdminRegistry, ManagedModel};
use crate::relations::{current_members, lookup_page, posted_members, scoped_membership};
use crate::rendering::render;
use axum::body::Bytes;
use axum::extract::{Extension, Path, Query, RawQuery, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use fexu_auth::{AppState, AuthConfig, AuthError, AuthUser, CookieSameSite};
use fexu_core::{ColumnKind, ColumnSpec, FexuError};
use fexu_db::driver::{TransactionTrait, Value as DbValue};
use fexu_db::{date_buckets, delete_record, get_record, query_labels, update_record};
use fexu_forms::{
    reject_unknown, require_confirmation, required_bool, required_text, FormValues, TextSpec,
};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use uuid::Uuid;

pub fn routes(router: Router<AppState>, registry: AdminRegistry) -> Router<AppState> {
    router.merge(
        Router::new()
            .route("/admin", get(dashboard))
            .route("/admin/", get(dashboard))
            .route("/admin/login", get(login_form).post(login_submit))
            .route("/admin/logout", post(logout))
            .route("/admin/preferences", post(preferences))
            .route("/admin/account", get(account).post(account_save))
            .route(
                "/admin/account/password",
                get(account_password).post(account_password_save),
            )
            .route("/admin/settings", get(settings_page).post(settings_save))
            .route("/admin/brand/favicon", get(brand_favicon))
            .route("/admin/brand/apple-touch", get(brand_apple))
            .route("/admin/search", get(admin_search))
            .route("/admin/boards/config", get(board_config))
            .route("/admin/boards", post(board_create))
            .route("/admin/boards/{id}", get(board_show))
            .route("/admin/boards/{id}/edit", get(board_edit))
            .route("/admin/boards/{id}/choose", post(board_choose))
            .route("/admin/boards/{id}/delete", post(board_delete))
            .route("/admin/boards/{id}/home", post(board_home))
            .route("/admin/boards/{id}/move", post(board_move))
            .route("/admin/boards/{id}/preview", post(widget_preview))
            .route("/admin/boards/{id}/widgets", post(widget_create))
            .route("/admin/boards/{id}/widgets/{widget}", post(widget_update))
            .route(
                "/admin/boards/{id}/widgets/{widget}/move",
                post(widget_move),
            )
            .route(
                "/admin/boards/{id}/widgets/{widget}/place",
                post(widget_place),
            )
            .route(
                "/admin/boards/{id}/widgets/{widget}/delete",
                post(widget_delete),
            )
            .route("/admin/audit", get(audit_page))
            .route("/admin/models", get(model_index))
            .route("/admin/models/{model}", get(model_list))
            .route("/admin/models/{model}/bulk", post(model_bulk))
            .route(
                "/admin/models/{model}/bulk/{draft}",
                get(bulk_confirm).post(bulk_apply),
            )
            .route("/admin/models/{model}/draft", post(model_draft))
            .route(
                "/admin/models/{model}/new",
                get(model_new).post(model_create),
            )
            .route("/admin/models/{model}/pick/{field}", get(model_pick))
            .route(
                "/admin/models/{model}/{id}/pick/{field}",
                get(model_record_pick),
            )
            .route("/admin/models/{model}/{id}/draft", post(model_record_draft))
            .route(
                "/admin/models/{model}/{id}/inline/{inline}/new",
                get(inline_new).post(inline_create),
            )
            .route(
                "/admin/models/{model}/{id}",
                get(model_detail).post(model_update),
            )
            .route(
                "/admin/models/{model}/{id}/edit",
                get(model_detail).post(model_update),
            )
            .route(
                "/admin/models/{model}/{id}/delete",
                get(model_delete).post(model_remove),
            )
            .route("/admin/models/{model}/{id}/file/{field}", get(record_file))
            .route("/admin/models/{model}/{id}/row", post(row_save))
            .route("/admin/directory/users", get(users))
            .route(
                "/admin/directory/users/new",
                get(user_new).post(user_create),
            )
            .route(
                "/admin/directory/users/{id}",
                get(user_detail).post(user_update),
            )
            .route(
                "/admin/directory/users/{id}/edit",
                get(user_edit).post(user_update),
            )
            .route("/admin/directory/users/{id}/password", post(user_password))
            .route("/admin/directory/users/{id}/active", post(user_active))
            .route("/admin/directory/users/{id}/staff", post(user_staff))
            .route("/admin/directory/roles", get(roles).post(role_create))
            .route(
                "/admin/directory/roles/new",
                get(role_new).post(role_create),
            )
            .route("/admin/directory/roles/{id}", get(role_detail))
            .route("/admin/directory/permissions", get(permissions_page))
            .route(
                "/admin/directory/roles/{role}/permissions",
                post(permission_change),
            )
            .route("/admin/directory/assignments", post(assignment))
            .route(
                "/admin/pages/{key}",
                get(crate::pages::show).post(crate::pages::submit),
            )
            .layer(Extension(registry)),
    )
}

async fn login_form(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let locale = locale_of(
        &headers,
        registry.branding().default_locale.as_str(),
        registry.branding().fallback_locale.as_str(),
    );
    let token = login_token(&headers);
    let mut response = page(
        &registry,
        "login.html",
        &locale,
        theme_of(&headers),
        catalog(&locale)["login"].as_str().unwrap_or("Login"),
        false,
        false,
        "",
        "",
        "/admin/login",
        "",
        "",
        json!({
            "login_csrf": token,
            "next": safe_next(query.get("next").map(String::as_str).unwrap_or("/admin")).unwrap_or("/admin"),
            "email": ""
        }),
    );
    append_cookie(
        &mut response,
        &login_cookie(state.auth.as_ref().map(|auth| auth.config()), &token),
    );
    same_origin_document(&mut response);
    response
}

async fn login_submit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    body: Bytes,
) -> Response {
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let form = parse_form(&body);
    let Some(auth) = state.auth.as_ref() else {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::FORBIDDEN,
            "forbidden",
            false,
            "",
        )
        .await;
    };
    let supplied = form.get("csrf_token").map(String::as_str).unwrap_or("");
    if let Err(reason) = login_gate(auth, &headers, supplied) {
        log_login_rejection(&headers, reason);
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::FORBIDDEN,
            "csrf",
            false,
            "",
        )
        .await;
    }
    let next =
        safe_next(form.get("next").map(String::as_str).unwrap_or("/admin")).unwrap_or("/admin");
    let anonymous = cookie(&headers, &auth.config().anonymous_cookie_name);
    let Some(password) = form.get("password").map(String::as_str) else {
        let token = login_token(&headers);
        let mut response = page(
            &registry,
            "login.html",
            &locale,
            theme_of(&headers),
            catalog(&locale)["login"].as_str().unwrap_or("Login"),
            false,
            false,
            "",
            "",
            "/admin/login",
            "",
            &validation_text(&locale, "Invalid email or password"),
            json!({ "login_csrf": token, "next": next, "email": form.get("email").cloned().unwrap_or_default() }),
        );
        *response.status_mut() = StatusCode::UNAUTHORIZED;
        append_cookie(&mut response, &login_cookie(Some(auth.config()), &token));
        same_origin_document(&mut response);
        return response;
    };
    match auth
        .login_replacing(
            form.get("email").map(String::as_str).unwrap_or(""),
            password,
            &auth.client_key(&headers, "html"),
            anonymous.as_deref(),
        )
        .await
    {
        Ok(issued) => {
            let mut response = redirect(next);
            append_cookie(&mut response, &issued.cookie);
            for cookie in &issued.clear_cookies {
                append_cookie(&mut response, cookie);
            }
            append_cookie(
                &mut response,
                &clear_cookie("fexu_login_csrf", auth.config()),
            );
            response
        }
        Err(AuthError::RateLimited) => {
            error_page(
                &state,
                &headers,
                &registry,
                &locale,
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                false,
                "",
            )
            .await
        }
        Err(AuthError::InvalidCredentials) => {
            let token = login_token(&headers);
            let mut response = page(
                &registry,
                "login.html",
                &locale,
                theme_of(&headers),
                catalog(&locale)["login"].as_str().unwrap_or("Login"),
                false,
                false,
                "",
                "",
                "/admin/login",
                "",
                &validation_text(&locale, "Invalid email or password"),
                json!({ "login_csrf": token, "next": next, "email": form.get("email").cloned().unwrap_or_default() }),
            );
            *response.status_mut() = StatusCode::UNAUTHORIZED;
            append_cookie(&mut response, &login_cookie(Some(auth.config()), &token));
            same_origin_document(&mut response);
            response
        }
        Err(_) => {
            error_page(
                &state,
                &headers,
                &registry,
                &locale,
                StatusCode::FORBIDDEN,
                "forbidden",
                false,
                "",
            )
            .await
        }
    }
}

async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    body: Bytes,
) -> Response {
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let Some(auth) = state.auth.as_ref() else {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::FORBIDDEN,
            "forbidden",
            false,
            "",
        )
        .await;
    };
    let form = parse_form(&body);
    if auth
        .authenticate_csrf(
            &headers,
            &Method::POST,
            form.get("csrf_token").map(String::as_str).unwrap_or(""),
        )
        .await
        .is_err()
    {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::FORBIDDEN,
            "csrf",
            true,
            "",
        )
        .await;
    }
    let token = cookie(&headers, &auth.config().cookie_name);
    let cleared = auth.logout(token.as_deref()).await.unwrap_or_default();
    let mut response = redirect("/admin/login");
    if !cleared.is_empty() {
        append_cookie(&mut response, &cleared);
    }
    response
}

async fn preferences(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    body: Bytes,
) -> Response {
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    if require_post(&state, &headers, &body).await.is_err() {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::FORBIDDEN,
            "csrf",
            true,
            "",
        )
        .await;
    }
    let form = parse_form(&body);
    let next =
        safe_next(form.get("next").map(String::as_str).unwrap_or("/admin")).unwrap_or("/admin");
    let chosen = match form.get("locale").map(String::as_str).unwrap_or("") {
        "de" => "de",
        "en" => "en",
        _ => locale.as_str(),
    };
    let theme = match form.get("theme").map(String::as_str).unwrap_or("system") {
        "light" => "light",
        "dark" => "dark",
        _ => "system",
    };
    let config = state.auth.as_ref().map(|auth| auth.config());
    let mut response = redirect(next);
    append_cookie(
        &mut response,
        &preference_cookie("fexu_locale", chosen, config),
    );
    append_cookie(
        &mut response,
        &preference_cookie("fexu_theme", theme, config),
    );
    response
}

async fn dashboard(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let user = match require_get(&state, &headers, &registry, "/admin").await {
        Ok(user) => user,
        Err(response) => return response,
    };
    let Some(_auth) = state.auth.as_ref() else {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::FORBIDDEN,
            "forbidden",
            false,
            "",
        )
        .await;
    };
    let listed = match crate::boards::list_boards(&state.db).await {
        Ok(rows) => rows,
        Err(_) => {
            return error_page(
                &state,
                &headers,
                &registry,
                &locale,
                StatusCode::INTERNAL_SERVER_ERROR,
                "board_read_error",
                true,
                &user.email,
            )
            .await;
        }
    };
    if let Some(board) = listed.iter().find(|board| board.home).or(listed.first()) {
        return redirect(&format!("/admin/boards/{}", board.id));
    }
    if user.is_superuser {
        return redirect("/admin/boards/config");
    }
    let csrf = session_csrf(state.auth.as_ref().unwrap(), &headers).await;
    page_as(
        &state,
        &user,
        &registry,
        "dashboard.html",
        &locale,
        theme_of(&headers),
        catalog(&locale)["dashboards"]
            .as_str()
            .unwrap_or("Dashboards"),
        true,
        user.is_superuser,
        &user.email,
        &csrf,
        "/admin",
        &flash(
            &locale,
            query.get("notice").map(String::as_str).unwrap_or(""),
        ),
        "",
        json!({}),
    )
    .await
}

async fn account(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let user = match require_get(&state, &headers, &registry, "/admin/account").await {
        Ok(user) => user,
        Err(response) => return response,
    };
    let Some(auth) = state.auth.as_ref() else {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::FORBIDDEN,
            "forbidden",
            true,
            &user.email,
        )
        .await;
    };
    let csrf = session_csrf(auth, &headers).await;
    let display_name = auth.display_name(user.id).await.unwrap_or_default();
    page_as(
        &state,
        &user,
        &registry,
        "account.html",
        &locale,
        theme_of(&headers),
        catalog(&locale)["account"].as_str().unwrap_or("Account"),
        true,
        user.is_superuser,
        &user.email,
        &csrf,
        "/admin/account",
        &flash(
            &locale,
            query.get("notice").map(String::as_str).unwrap_or(""),
        ),
        "",
        json!({
            "user_email": user.email,
            "display_name": display_name,
            "profile_error": ""
        }),
    )
    .await
}

async fn account_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    body: Bytes,
) -> Response {
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let user = match require_post(&state, &headers, &body).await {
        Ok(user) => user,
        Err(_) => {
            return error_page(
                &state,
                &headers,
                &registry,
                &locale,
                StatusCode::FORBIDDEN,
                "csrf",
                true,
                "",
            )
            .await
        }
    };
    let Some(auth) = state.auth.as_ref() else {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::FORBIDDEN,
            "forbidden",
            true,
            &user.email,
        )
        .await;
    };
    let form = parse_form(&body);
    let name = form.get("name").map(String::as_str).unwrap_or("");
    let email = form
        .get("email")
        .map(String::as_str)
        .unwrap_or(user.email.as_str());
    let profile_error = if fexu_auth::clean_display_name(name).is_err()
        || fexu_auth::normalize_email(email).is_err()
    {
        catalog(&locale)
            .get("profile_invalid")
            .and_then(Value::as_str)
            .unwrap_or("The profile is invalid.")
            .to_owned()
    } else {
        String::new()
    };
    if !profile_error.is_empty() {
        return account_page(
            PageCtx {
                state: &state,
                headers: &headers,
                registry: &registry,
                locale: &locale,
                user: &user,
            },
            AccountForm {
                notice: "",
                error: &profile_error,
                name,
                email,
            },
        )
        .await;
    }
    if auth.set_display_name(user.id, name).await.is_err() {
        return account_page(
            PageCtx {
                state: &state,
                headers: &headers,
                registry: &registry,
                locale: &locale,
                user: &user,
            },
            AccountForm {
                notice: "",
                error: catalog(&locale)
                    .get("profile_invalid")
                    .and_then(Value::as_str)
                    .unwrap_or("The profile is invalid."),
                name,
                email,
            },
        )
        .await;
    }
    if auth.update_own_email(user.id, email).await.is_err() {
        return account_page(
            PageCtx {
                state: &state,
                headers: &headers,
                registry: &registry,
                locale: &locale,
                user: &user,
            },
            AccountForm {
                notice: "",
                error: catalog(&locale)
                    .get("email_taken")
                    .and_then(Value::as_str)
                    .unwrap_or("That email is already registered."),
                name,
                email,
            },
        )
        .await;
    }
    redirect("/admin/account?notice=updated")
}

struct PageCtx<'a> {
    state: &'a AppState,
    headers: &'a HeaderMap,
    registry: &'a AdminRegistry,
    locale: &'a str,
    user: &'a AuthUser,
}

fn screen<'a>(
    state: &'a AppState,
    headers: &'a HeaderMap,
    registry: &'a AdminRegistry,
    locale: &'a str,
    user: &'a AuthUser,
) -> PageCtx<'a> {
    PageCtx {
        state,
        headers,
        registry,
        locale,
        user,
    }
}

struct AccountForm<'a> {
    notice: &'a str,
    error: &'a str,
    name: &'a str,
    email: &'a str,
}

async fn account_page(page: PageCtx<'_>, form: AccountForm<'_>) -> Response {
    let PageCtx {
        state,
        headers,
        registry,
        user,
        locale,
    } = page;
    let AccountForm {
        notice,
        error: profile_error,
        name,
        email,
    } = form;
    let csrf = if let Some(auth) = state.auth.as_ref() {
        session_csrf(auth, headers).await
    } else {
        String::new()
    };
    let mut response = page_as(
        state,
        user,
        registry,
        "account.html",
        locale,
        theme_of(headers),
        catalog(locale)["account"].as_str().unwrap_or("Account"),
        true,
        user.is_superuser,
        email,
        &csrf,
        "/admin/account",
        notice,
        "",
        json!({
            "user_email": email,
            "display_name": name,
            "profile_error": profile_error
        }),
    )
    .await;
    if !profile_error.is_empty() {
        *response.status_mut() = StatusCode::UNPROCESSABLE_ENTITY;
    }
    response
}

async fn account_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
) -> Response {
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let user = match require_get(&state, &headers, &registry, "/admin/account/password").await {
        Ok(user) => user,
        Err(response) => return response,
    };
    password_page(&state, &headers, &registry, &user, &locale, "").await
}

async fn account_password_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    body: Bytes,
) -> Response {
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let user = match require_post(&state, &headers, &body).await {
        Ok(user) => user,
        Err(_) => {
            return error_page(
                &state,
                &headers,
                &registry,
                &locale,
                StatusCode::FORBIDDEN,
                "csrf",
                true,
                "",
            )
            .await
        }
    };
    let Some(auth) = state.auth.as_ref() else {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::FORBIDDEN,
            "forbidden",
            true,
            &user.email,
        )
        .await;
    };
    let form = parse_form(&body);
    let Some((current, password)) = posted_password_change(&form) else {
        return password_page(
            &state,
            &headers,
            &registry,
            &user,
            &locale,
            catalog(&locale)
                .get("password_mismatch")
                .and_then(Value::as_str)
                .unwrap_or("The passwords do not match."),
        )
        .await;
    };
    match auth.change_password(&user, current, password).await {
        Ok(cleared) => {
            let mut response = redirect("/admin/login");
            append_cookie(&mut response, &cleared);
            response
        }
        Err(AuthError::InvalidCredentials) => {
            password_page(
                &state,
                &headers,
                &registry,
                &user,
                &locale,
                catalog(&locale)
                    .get("password_current_invalid")
                    .and_then(Value::as_str)
                    .unwrap_or("The current password is wrong."),
            )
            .await
        }
        Err(_) => {
            password_page(
                &state,
                &headers,
                &registry,
                &user,
                &locale,
                catalog(&locale)
                    .get("password_invalid")
                    .and_then(Value::as_str)
                    .unwrap_or("The password must contain 12 to 128 characters."),
            )
            .await
        }
    }
}

fn posted_password_change(form: &HashMap<String, String>) -> Option<(&str, &str)> {
    let current = form
        .get("current_password")
        .map(String::as_str)
        .filter(|value| !value.is_empty())?;
    let password = form
        .get("password")
        .map(String::as_str)
        .filter(|value| !value.is_empty())?;
    let confirm = form.get("password_confirm").map(String::as_str)?;
    (password == confirm).then_some((current, password))
}

async fn password_page(
    state: &AppState,
    headers: &HeaderMap,
    registry: &AdminRegistry,
    user: &AuthUser,
    locale: &str,
    password_error: &str,
) -> Response {
    let csrf = if let Some(auth) = state.auth.as_ref() {
        session_csrf(auth, headers).await
    } else {
        String::new()
    };
    let mut response = page_as(
        state,
        user,
        registry,
        "password.html",
        locale,
        theme_of(headers),
        catalog(locale)["set_password"]
            .as_str()
            .unwrap_or("Set a new password"),
        true,
        user.is_superuser,
        &user.email,
        &csrf,
        "/admin/account/password",
        "",
        "",
        json!({ "password_error": password_error }),
    )
    .await;
    if !password_error.is_empty() {
        *response.status_mut() = StatusCode::UNPROCESSABLE_ENTITY;
    }
    response
}

async fn settings_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    crate::project::ensure(&registry, &state.db).await;
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let user = match require_get(&state, &headers, &registry, "/admin/settings").await {
        Ok(user) => user,
        Err(response) => return response,
    };
    let csrf = if let Some(auth) = state.auth.as_ref() {
        session_csrf(auth, &headers).await
    } else {
        String::new()
    };
    page_as(
        &state,
        &user,
        &registry,
        "settings.html",
        &locale,
        theme_of(&headers),
        catalog(&locale)["settings"].as_str().unwrap_or("Settings"),
        true,
        user.is_superuser,
        &user.email,
        &csrf,
        "/admin/settings",
        &flash(
            &locale,
            query.get("notice").map(String::as_str).unwrap_or(""),
        ),
        "",
        json!({ "project_editor": user.is_superuser, "project_error": "" }),
    )
    .await
}

struct SettingsPost {
    fields: HashMap<String, String>,
    favicon: Vec<u8>,
    apple: Vec<u8>,
}

fn settings_post(headers: &HeaderMap, body: &Bytes) -> Result<SettingsPost, ()> {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    if !content_type
        .to_ascii_lowercase()
        .starts_with("multipart/form-data")
    {
        return Ok(SettingsPost {
            fields: parse_form(body),
            favicon: Vec::new(),
            apple: Vec::new(),
        });
    }
    let parts = fexu_storage::read_multipart(content_type, body).map_err(|_| ())?;
    let mut fields = HashMap::new();
    let mut favicon = Vec::new();
    let mut apple = Vec::new();
    for part in parts {
        if part.filename.is_some() && part.name == "favicon" {
            favicon = part.bytes;
            continue;
        }
        if part.filename.is_some() && part.name == "apple_touch_icon" {
            apple = part.bytes;
            continue;
        }
        if part.filename.is_none() {
            if let Ok(text) = String::from_utf8(part.bytes) {
                fields.insert(part.name, text);
            }
        }
    }
    Ok(SettingsPost {
        fields,
        favicon,
        apple,
    })
}

async fn project_rejected(
    state: &AppState,
    headers: &HeaderMap,
    registry: &AdminRegistry,
    user: &AuthUser,
    locale: &str,
    theme: &str,
    fields: &[(&str, &str)],
) -> Response {
    let csrf = if let Some(auth) = state.auth.as_ref() {
        session_csrf(auth, headers).await
    } else {
        String::new()
    };
    let mut extra = json!({
        "project_editor": true,
        "project_error": catalog(locale).get("project_invalid").and_then(Value::as_str).unwrap_or("The project details are invalid.")
    });
    if let Some(object) = extra.as_object_mut() {
        for (key, value) in fields {
            let name = match *key {
                "primary_color" => "project_color",
                "default_locale" => "project_locale",
                "fallback_locale" => "project_fallback",
                "currency" => "project_currency",
                other => other,
            };
            object.insert((*name).to_owned(), json!(value));
        }
    }
    let mut response = page_as(
        state,
        user,
        registry,
        "settings.html",
        locale,
        theme,
        catalog(locale)["settings"].as_str().unwrap_or("Settings"),
        true,
        user.is_superuser,
        &user.email,
        &csrf,
        "/admin/settings",
        "",
        "",
        extra,
    )
    .await;
    *response.status_mut() = StatusCode::UNPROCESSABLE_ENTITY;
    let config = state.auth.as_ref().map(|auth| auth.config());
    append_cookie(
        &mut response,
        &preference_cookie("fexu_locale", locale, config),
    );
    append_cookie(
        &mut response,
        &preference_cookie("fexu_theme", theme, config),
    );
    response
}

async fn brand_favicon(State(state): State<AppState>) -> Response {
    brand_icon(&state, true).await
}

async fn brand_apple(State(state): State<AppState>) -> Response {
    brand_icon(&state, false).await
}

async fn brand_icon(state: &AppState, favicon: bool) -> Response {
    let file = match crate::project::load(&state.db).await {
        Ok(Some(record)) => {
            if favicon {
                record.favicon_file
            } else {
                record.apple_file
            }
        }
        _ => None,
    };
    let Some((kind, bytes)) = file else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let content_type = match kind.as_str() {
        "image/png" => "image/png",
        "image/jpeg" => "image/jpeg",
        "image/gif" => "image/gif",
        "image/webp" => "image/webp",
        "image/x-icon" => "image/x-icon",
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    let mut response = Response::new(axum::body::Body::from(bytes));
    *response.status_mut() = StatusCode::OK;
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=86400"),
    );
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

async fn settings_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    body: Bytes,
) -> Response {
    crate::project::ensure(&registry, &state.db).await;
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let user = match require_post(&state, &headers, &body).await {
        Ok(user) => user,
        Err(()) => {
            return error_page(
                &state,
                &headers,
                &registry,
                &locale,
                StatusCode::FORBIDDEN,
                "csrf",
                true,
                "",
            )
            .await
        }
    };
    let posted = match settings_post(&headers, &body) {
        Ok(posted) => posted,
        Err(()) => {
            let theme = theme_of(&headers);
            return project_rejected(&state, &headers, &registry, &user, &locale, &theme, &[])
                .await;
        }
    };
    let form = &posted.fields;
    let chosen = match form.get("locale").map(String::as_str).unwrap_or("") {
        "de" => "de",
        "en" => "en",
        _ => locale.as_str(),
    };
    let current_theme = theme_of(&headers);
    let theme = match form.get("theme").map(String::as_str).unwrap_or("") {
        "light" => "light",
        "dark" => "dark",
        "system" => "system",
        _ => current_theme.as_str(),
    };
    let config = state.auth.as_ref().map(|auth| auth.config());
    if user.is_superuser {
        let fields = [
            (
                "site_title",
                form.get("site_title").map(String::as_str).unwrap_or(""),
            ),
            (
                "site_header",
                form.get("site_header").map(String::as_str).unwrap_or(""),
            ),
            (
                "app_name",
                form.get("app_name").map(String::as_str).unwrap_or(""),
            ),
            (
                "primary_color",
                form.get("primary_color").map(String::as_str).unwrap_or(""),
            ),
            (
                "default_locale",
                form.get("default_locale").map(String::as_str).unwrap_or(""),
            ),
            (
                "fallback_locale",
                form.get("fallback_locale")
                    .map(String::as_str)
                    .unwrap_or(""),
            ),
            (
                "currency",
                form.get("currency").map(String::as_str).unwrap_or(""),
            ),
        ];
        let Some(mut record) = crate::project::parse(&fields) else {
            return project_rejected(&state, &headers, &registry, &user, chosen, theme, &fields)
                .await;
        };
        let existing = crate::project::load(&state.db).await.ok().flatten();
        let (old_favicon, old_favicon_file, old_apple, old_apple_file) = match existing {
            Some(existing) => (
                existing.icons.favicon,
                existing.favicon_file,
                existing.icons.apple_touch_icon,
                existing.apple_file,
            ),
            None => (String::new(), None, String::new(), None),
        };
        let Some(favicon_choice) = crate::project::choose_icon(
            form.get("favicon_clear").map(String::as_str) == Some("1"),
            &posted.favicon,
        ) else {
            return project_rejected(&state, &headers, &registry, &user, chosen, theme, &fields)
                .await;
        };
        let Some(apple_choice) = crate::project::choose_icon(
            form.get("apple_clear").map(String::as_str) == Some("1"),
            &posted.apple,
        ) else {
            return project_rejected(&state, &headers, &registry, &user, chosen, theme, &fields)
                .await;
        };
        let (href, file) =
            crate::project::apply_icon(&old_favicon, old_favicon_file, favicon_choice, "favicon");
        record.icons.favicon = href;
        record.favicon_file = file;
        let (href, file) =
            crate::project::apply_icon(&old_apple, old_apple_file, apple_choice, "apple-touch");
        record.icons.apple_touch_icon = href;
        record.apple_file = file;
        if crate::project::save(&state.db, &record).await.is_err() {
            return error_page(
                &state,
                &headers,
                &registry,
                chosen,
                StatusCode::INTERNAL_SERVER_ERROR,
                "forbidden",
                true,
                &user.email,
            )
            .await;
        }
        registry.apply_project(record.branding, record.icons);
        registry.mark_project_loaded();
    }
    let mut response = redirect("/admin/settings?notice=updated");
    append_cookie(
        &mut response,
        &preference_cookie("fexu_locale", chosen, config),
    );
    append_cookie(
        &mut response,
        &preference_cookie("fexu_theme", theme, config),
    );
    response
}

async fn admin_search(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let user = match require_get(&state, &headers, &registry, "/admin/search").await {
        Ok(user) => user,
        Err(response) => return response,
    };
    let query = params
        .get("q")
        .map(|value| value.trim().to_owned())
        .unwrap_or_default();
    if query.chars().count() > 200 {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::UNPROCESSABLE_ENTITY,
            "forbidden",
            true,
            &user.email,
        )
        .await;
    }
    let csrf = session_csrf(state.auth.as_ref().unwrap(), &headers).await;
    let groups = if query.is_empty() {
        Vec::new()
    } else {
        match search_groups(&state, &headers, &registry, &user, &locale, &query).await {
            Ok(groups) => groups,
            Err(response) => return *response,
        }
    };
    page_as(
        &state,
        &user,
        &registry,
        "search.html",
        &locale,
        theme_of(&headers),
        &text(&locale, "search_title"),
        true,
        user.is_superuser,
        &user.email,
        &csrf,
        "/admin/search",
        "",
        "",
        json!({ "query": query, "groups": groups }),
    )
    .await
}

async fn search_groups(
    state: &AppState,
    headers: &HeaderMap,
    registry: &AdminRegistry,
    user: &AuthUser,
    locale: &str,
    query: &str,
) -> Result<Vec<Value>, Box<Response>> {
    let needle = query.to_lowercase();
    let Some(auth) = state.auth.as_ref() else {
        return Err(Box::new(
            error_page(
                state,
                headers,
                registry,
                locale,
                StatusCode::FORBIDDEN,
                "forbidden",
                true,
                &user.email,
            )
            .await,
        ));
    };
    let mut groups = Vec::new();
    let mut pages = Vec::new();
    let boards = match crate::boards::list_boards(&state.db).await {
        Ok(rows) => rows,
        Err(_) => {
            return Err(Box::new(
                error_page(
                    state,
                    headers,
                    registry,
                    locale,
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "board_read_error",
                    true,
                    &user.email,
                )
                .await,
            ));
        }
    };
    for board in boards {
        if board.name.to_lowercase().contains(&needle) {
            pages.push(json!({
                "text": board.name,
                "href": format!("/admin/boards/{}", board.id)
            }));
        }
    }
    for (key, href) in [
        ("history", "/admin/audit"),
        ("users", "/admin/directory/users"),
        ("roles", "/admin/directory/roles"),
    ] {
        if key != "history" && !user.is_superuser {
            continue;
        }
        let label = text(locale, key);
        if label.to_lowercase().contains(&needle) {
            pages.push(json!({ "text": label, "href": href }));
        }
    }
    for view in registry.views() {
        if !view.in_navigation() {
            continue;
        }
        let title = view.title(locale);
        if !title.to_lowercase().contains(&needle) {
            continue;
        }
        let allowed = match auth.holds(user, view.permission_code()).await {
            Ok(allowed) => allowed,
            Err(_) => {
                return Err(Box::new(
                    error_page(
                        state,
                        headers,
                        registry,
                        locale,
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "forbidden",
                        true,
                        &user.email,
                    )
                    .await,
                ));
            }
        };
        if allowed {
            pages.push(json!({
                "text": title,
                "href": format!("/admin/pages/{}", view.key())
            }));
        }
    }
    if !pages.is_empty() {
        groups.push(json!({
            "label": text(locale, "search_pages"),
            "href": "",
            "hits": pages,
            "more": false
        }));
    }
    if user.is_superuser {
        let people = match auth.list_users().await {
            Ok(rows) => rows,
            Err(_) => {
                return Err(Box::new(
                    error_page(
                        state,
                        headers,
                        registry,
                        locale,
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "forbidden",
                        true,
                        &user.email,
                    )
                    .await,
                ));
            }
        };
        let mut hits = Vec::new();
        for person in people {
            if person.email.to_lowercase().contains(&needle) {
                hits.push(json!({
                    "text": person.email,
                    "href": format!("/admin/directory/users/{}", person.id)
                }));
            }
        }
        let more = hits.len() > 5;
        hits.truncate(5);
        if !hits.is_empty() {
            groups.push(json!({
                "label": text(locale, "users"),
                "href": format!("/admin/directory/users?q={}", url_encode(query)),
                "hits": hits,
                "more": more
            }));
        }
        let roles = match auth.list_roles().await {
            Ok(rows) => rows,
            Err(_) => {
                return Err(Box::new(
                    error_page(
                        state,
                        headers,
                        registry,
                        locale,
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "forbidden",
                        true,
                        &user.email,
                    )
                    .await,
                ));
            }
        };
        let mut hits = Vec::new();
        for (id, name) in roles {
            if name.to_lowercase().contains(&needle) {
                hits.push(json!({
                    "text": name,
                    "href": format!("/admin/directory/roles/{id}")
                }));
            }
        }
        let more = hits.len() > 5;
        hits.truncate(5);
        if !hits.is_empty() {
            groups.push(json!({
                "label": text(locale, "roles"),
                "href": "/admin/directory/roles",
                "hits": hits,
                "more": more
            }));
        }
    }
    for model in registry.models() {
        if list_blocked(model.as_ref()) {
            continue;
        }
        match permit(auth, user, model.meta(), "read").await {
            Ok(()) => {}
            Err(FexuError::Forbidden | FexuError::Denied(_) | FexuError::Unauthorized(_)) => {
                continue
            }
            Err(_) => {
                return Err(Box::new(
                    error_page(
                        state,
                        headers,
                        registry,
                        locale,
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "forbidden",
                        true,
                        &user.email,
                    )
                    .await,
                ));
            }
        }
        let key = model.meta().key;
        let app_key = registry.model_app(key).unwrap_or("");
        let app_label = registry
            .app_catalog()
            .iter()
            .find(|(name, _)| name == app_key)
            .map(|(_, label)| label.as_str())
            .unwrap_or("");
        let named = model.meta().label.to_lowercase().contains(&needle)
            || (!app_label.is_empty() && app_label.to_lowercase().contains(&needle));
        let fields = registry.field_slice(model.meta());
        let search_columns: Vec<&str> = fields
            .iter()
            .filter(|field| field.searchable && !field.hidden && !sensitive(field.name))
            .map(|field| field.name)
            .collect();
        let href = if search_columns.is_empty() {
            format!("/admin/models/{key}")
        } else {
            format!("/admin/models/{key}?q={}", url_encode(query))
        };
        if search_columns.is_empty() {
            if named {
                groups.push(json!({
                    "label": model.meta().label,
                    "href": href,
                    "hits": [],
                    "more": false
                }));
            }
            continue;
        }
        let (sort, descending) =
            resolved_sort(fields, registry.ordering_of(model.meta()), &HashMap::new())
                .unwrap_or_else(|_| ("id".to_owned(), false));
        let scope = access_scope(model.as_ref(), user, false);
        let listed = model
            .page(
                &state.db,
                &search_columns,
                Some(query),
                &sort,
                descending,
                &[],
                &[],
                0,
                5,
                &scope,
            )
            .await;
        let (rows, _, total) = match listed {
            Ok(value) => value,
            Err(_) => {
                return Err(Box::new(
                    error_page(
                        state,
                        headers,
                        registry,
                        locale,
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "forbidden",
                        true,
                        &user.email,
                    )
                    .await,
                ));
            }
        };
        if rows.is_empty() && !named {
            continue;
        }
        let relations = match relation_names(registry, state, user, model.columns(), &rows).await {
            Ok(relations) => relations,
            Err(_) => {
                return Err(Box::new(
                    error_page(
                        state,
                        headers,
                        registry,
                        locale,
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "forbidden",
                        true,
                        &user.email,
                    )
                    .await,
                ));
            }
        };
        let links = link_names(
            fields,
            model.columns(),
            registry.list_order_of(model.meta()),
        );
        let label_field = links.first().copied().unwrap_or("id");
        let currency = registry.branding().currency;
        let mut hits = Vec::new();
        for row in &rows {
            let Some(id) = row.get("id").and_then(Value::as_str) else {
                continue;
            };
            let mut lead = false;
            let cell = present_cell(
                locale,
                currency.as_str(),
                model.columns(),
                label_field,
                row.get(label_field).unwrap_or(&Value::Null),
                &relations,
                user.is_superuser,
                &mut lead,
            );
            let text = cell
                .get("text")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty() && *value != "—")
                .unwrap_or(id);
            hits.push(json!({
                "text": text,
                "href": format!("/admin/models/{key}/{id}")
            }));
        }
        groups.push(json!({
            "label": model.meta().label,
            "href": href,
            "hits": hits,
            "more": total > 5
        }));
    }
    Ok(groups)
}

async fn board_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
) -> Response {
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let user = match require_get(&state, &headers, &registry, "/admin/boards/config").await {
        Ok(user) => user,
        Err(response) => return response,
    };
    if !user.is_superuser {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::FORBIDDEN,
            "forbidden",
            true,
            &user.email,
        )
        .await;
    }
    let listed = match crate::boards::list_boards(&state.db).await {
        Ok(rows) => rows,
        Err(_) => {
            return error_page(
                &state,
                &headers,
                &registry,
                &locale,
                StatusCode::INTERNAL_SERVER_ERROR,
                "board_read_error",
                true,
                &user.email,
            )
            .await;
        }
    };
    let boards = board_rows(listed);
    let Some(auth) = state.auth.as_ref() else {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::FORBIDDEN,
            "forbidden",
            true,
            &user.email,
        )
        .await;
    };
    let csrf = session_csrf(auth, &headers).await;
    page_as(
        &state,
        &user,
        &registry,
        "board_config.html",
        &locale,
        theme_of(&headers),
        &text(&locale, "board_config"),
        true,
        user.is_superuser,
        &user.email,
        &csrf,
        "/admin/boards/config",
        "",
        "",
        json!({ "boards": boards }),
    )
    .await
}

async fn board_create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    body: Bytes,
) -> Response {
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let user = match require_post(&state, &headers, &body).await {
        Ok(user) => user,
        Err(()) => {
            return error_page(
                &state,
                &headers,
                &registry,
                &locale,
                StatusCode::FORBIDDEN,
                "csrf",
                true,
                "",
            )
            .await;
        }
    };
    if !user.is_superuser {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::FORBIDDEN,
            "forbidden",
            true,
            &user.email,
        )
        .await;
    }
    let form = parse_form(&body);
    match crate::boards::create_board(
        &state.db,
        form.get("name").map(String::as_str).unwrap_or(""),
    )
    .await
    {
        Ok(id) => redirect(&format!("/admin/boards/{id}/edit")),
        Err(Ok(reject)) => {
            let Ok(rows) = crate::boards::list_boards(&state.db).await else {
                return error_page(
                    &state,
                    &headers,
                    &registry,
                    &locale,
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "board_read_error",
                    true,
                    &user.email,
                )
                .await;
            };
            let boards = board_rows(rows);
            board_alert(BoardAlert {
                page: screen(&state, &headers, &registry, &locale, &user),
                path: "/admin/boards/config",
                template: "board_config.html",
                title: &text(&locale, "board_config"),
                alert_key: board_reject_key(reject),
                extra: json!({ "boards": boards }),
            })
            .await
        }
        Err(Err(_)) => {
            error_page(
                &state,
                &headers,
                &registry,
                &locale,
                StatusCode::INTERNAL_SERVER_ERROR,
                "board_read_error",
                true,
                &user.email,
            )
            .await
        }
    }
}

async fn board_show(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let user = match require_get(&state, &headers, &registry, "/admin/boards").await {
        Ok(user) => user,
        Err(response) => return response,
    };
    let Some(id) = parse_uuid(&id) else {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    };
    let loaded = match crate::boards::load(&state.db, id).await {
        Ok(Some(loaded)) => loaded,
        Ok(None) => {
            return error_page(
                &state,
                &headers,
                &registry,
                &locale,
                StatusCode::NOT_FOUND,
                "not_found",
                true,
                &user.email,
            )
            .await;
        }
        Err(_) => {
            return error_page(
                &state,
                &headers,
                &registry,
                &locale,
                StatusCode::INTERNAL_SERVER_ERROR,
                "board_read_error",
                true,
                &user.email,
            )
            .await;
        }
    };
    let (applied, slicers, sliced) = match board_slices(
        &state, &headers, &user, &registry, &locale, &loaded.1, &params,
    )
    .await
    {
        Ok(value) => value,
        Err(response) => return *response,
    };
    let blocks = render_blocks(&state, &user, &registry, &locale, &loaded.1, &applied).await;
    let Some(auth) = state.auth.as_ref() else {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::FORBIDDEN,
            "forbidden",
            true,
            &user.email,
        )
        .await;
    };
    let csrf = session_csrf(auth, &headers).await;
    page_as(
        &state,
        &user,
        &registry,
        "board_show.html",
        &locale,
        theme_of(&headers),
        &loaded.0.name,
        true,
        user.is_superuser,
        &user.email,
        &csrf,
        &format!("/admin/boards/{id}"),
        "",
        "",
        json!({
            "board": { "id": id.to_string(), "name": loaded.0.name },
            "blocks": blocks,
            "slicers": slicers,
            "sliced": sliced
        }),
    )
    .await
}

async fn board_edit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    Path(id): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let user = match require_get(&state, &headers, &registry, "/admin/boards/config").await {
        Ok(user) => user,
        Err(response) => return response,
    };
    if !user.is_superuser {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::FORBIDDEN,
            "forbidden",
            true,
            &user.email,
        )
        .await;
    }
    let Some(id) = parse_uuid(&id) else {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    };
    render_board_edit(BoardEdit {
        page: screen(&state, &headers, &registry, &locale, &user),
        id,
        model_key: query.get("model").map(String::as_str),
        widget_id: query.get("widget").map(String::as_str),
        alert_key: "",
        preview: None,
        form_state: None,
    })
    .await
}

async fn board_choose(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let user = match require_post(&state, &headers, &body).await {
        Ok(user) => user,
        Err(()) => {
            return error_page(
                &state,
                &headers,
                &registry,
                &locale,
                StatusCode::FORBIDDEN,
                "csrf",
                true,
                "",
            )
            .await;
        }
    };
    if !user.is_superuser {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::FORBIDDEN,
            "forbidden",
            true,
            &user.email,
        )
        .await;
    }
    let Some(id) = parse_uuid(&id) else {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    };
    let form = parse_form(&body);
    let model = form.get("model").map(String::as_str).unwrap_or("");
    if registry.find(model).is_none() || !identifier(model) {
        return render_board_edit(BoardEdit {
            page: screen(&state, &headers, &registry, &locale, &user),
            id,
            model_key: None,
            widget_id: None,
            alert_key: "board_bad_model",
            preview: None,
            form_state: None,
        })
        .await;
    }
    redirect(&format!(
        "/admin/boards/{id}/edit?model={}",
        url_encode(model)
    ))
}

async fn board_delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let user = match posted_superuser(&state, &headers, &registry, &locale, &body).await {
        Ok(user) => user,
        Err(response) => return *response,
    };
    let Some(id) = parse_uuid(&id) else {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    };
    if crate::boards::delete_board(&state.db, id).await.is_err() {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::INTERNAL_SERVER_ERROR,
            "board_read_error",
            true,
            &user.email,
        )
        .await;
    }
    redirect("/admin/boards/config")
}

async fn widget_create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let user = match posted_superuser(&state, &headers, &registry, &locale, &body).await {
        Ok(user) => user,
        Err(response) => return *response,
    };
    let Some(id) = parse_uuid(&id) else {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    };
    let form = parse_form(&body);
    let model_key = form.get("model").map(String::as_str).unwrap_or("");
    let Some(model) = registry.find(model_key) else {
        return render_board_edit(BoardEdit {
            page: screen(&state, &headers, &registry, &locale, &user),
            id,
            model_key: None,
            widget_id: None,
            alert_key: "board_bad_model",
            preview: None,
            form_state: None,
        })
        .await;
    };
    commit_widget(WidgetCommit {
        page: screen(&state, &headers, &registry, &locale, &user),
        id,
        widget_id: None,
        model: &**model,
        form: &form,
    })
    .await
}

async fn widget_update(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    Path((id, widget)): Path<(String, String)>,
    body: Bytes,
) -> Response {
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let user = match posted_superuser(&state, &headers, &registry, &locale, &body).await {
        Ok(user) => user,
        Err(response) => return *response,
    };
    let Some(id) = parse_uuid(&id) else {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    };
    let Some(widget_id) = parse_uuid(&widget) else {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    };
    let loaded = match crate::boards::load(&state.db, id).await {
        Ok(Some(loaded)) => loaded,
        Ok(None) => {
            return error_page(
                &state,
                &headers,
                &registry,
                &locale,
                StatusCode::NOT_FOUND,
                "not_found",
                true,
                &user.email,
            )
            .await
        }
        Err(_) => {
            return error_page(
                &state,
                &headers,
                &registry,
                &locale,
                StatusCode::INTERNAL_SERVER_ERROR,
                "board_read_error",
                true,
                &user.email,
            )
            .await
        }
    };
    let Some(existing) = loaded.1.iter().find(|item| item.id == widget_id) else {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    };
    let Some(model) = registry.find(&existing.model_key) else {
        return render_board_edit(BoardEdit {
            page: screen(&state, &headers, &registry, &locale, &user),
            id,
            model_key: None,
            widget_id: Some(widget.as_str()),
            alert_key: "board_bad_model",
            preview: None,
            form_state: None,
        })
        .await;
    };
    commit_widget(WidgetCommit {
        page: screen(&state, &headers, &registry, &locale, &user),
        id,
        widget_id: Some(widget_id),
        model: &**model,
        form: &parse_form(&body),
    })
    .await
}

async fn widget_move(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    Path((id, widget)): Path<(String, String)>,
    body: Bytes,
) -> Response {
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let user = match posted_superuser(&state, &headers, &registry, &locale, &body).await {
        Ok(user) => user,
        Err(response) => return *response,
    };
    let Some(id) = parse_uuid(&id) else {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    };
    let Some(widget) = parse_uuid(&widget) else {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    };
    let up = parse_form(&body).get("direction").map(String::as_str) == Some("up");
    if crate::boards::move_widget(&state.db, id, widget, up)
        .await
        .is_err()
    {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::INTERNAL_SERVER_ERROR,
            "board_read_error",
            true,
            &user.email,
        )
        .await;
    }
    redirect(&format!("/admin/boards/{id}/edit"))
}

async fn board_home(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let user = match posted_superuser(&state, &headers, &registry, &locale, &body).await {
        Ok(user) => user,
        Err(response) => return *response,
    };
    let Some(id) = parse_uuid(&id) else {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    };
    let form = parse_form(&body);
    let enabled = form.get("home").map(String::as_str) == Some("1");
    if crate::boards::set_home(&state.db, id, enabled)
        .await
        .is_err()
    {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::INTERNAL_SERVER_ERROR,
            "board_read_error",
            true,
            &user.email,
        )
        .await;
    }
    let next = if form.get("next").map(String::as_str) == Some("config") {
        "/admin/boards/config".to_owned()
    } else {
        format!("/admin/boards/{id}/edit")
    };
    redirect(&next)
}

async fn board_move(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let user = match posted_superuser(&state, &headers, &registry, &locale, &body).await {
        Ok(user) => user,
        Err(response) => return *response,
    };
    let Some(id) = parse_uuid(&id) else {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    };
    let up = parse_form(&body).get("direction").map(String::as_str) == Some("up");
    if crate::boards::move_board(&state.db, id, up).await.is_err() {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::INTERNAL_SERVER_ERROR,
            "board_read_error",
            true,
            &user.email,
        )
        .await;
    }
    redirect("/admin/boards/config")
}

fn board_rows(rows: Vec<crate::boards::Board>) -> Vec<Value> {
    let total = rows.len();
    rows.into_iter()
        .enumerate()
        .map(|(index, board)| {
            json!({
                "id": board.id.to_string(),
                "name": board.name,
                "home": board.home,
                "first": index == 0,
                "last": index + 1 == total,
                "position": index + 1,
            })
        })
        .collect()
}

struct WidgetCommit<'a> {
    page: PageCtx<'a>,
    id: uuid::Uuid,
    widget_id: Option<uuid::Uuid>,
    model: &'a dyn ManagedModel,
    form: &'a HashMap<String, String>,
}

async fn commit_widget(commit: WidgetCommit<'_>) -> Response {
    let WidgetCommit {
        page,
        id,
        widget_id,
        model,
        form,
    } = commit;
    let PageCtx {
        state,
        headers,
        registry,
        locale,
        user,
    } = page;
    let model_key = model.meta().key;
    let draft = match widget_draft(model, form) {
        Ok(draft) => draft,
        Err(key) => {
            let widget_key = widget_id.map(|value| value.to_string());
            return render_board_edit(BoardEdit {
                page: screen(state, headers, registry, locale, user),
                id,
                model_key: Some(model_key),
                widget_id: widget_key.as_deref(),
                alert_key: key,
                preview: None,
                form_state: None,
            })
            .await;
        }
    };
    let stored = crate::boards::NewWidget {
        kind: &draft.kind,
        model_key,
        fields: &draft.fields,
        group_field: &draft.group_field,
        series_field: &draft.series_field,
        measure: &draft.measure,
        trend_field: &draft.trend_field,
        span: draft.span,
        title: &draft.title,
        group_is_date: crate::boards::group_is_date(model.columns(), &draft.group_field),
        trend_is_date: crate::boards::group_is_date(model.columns(), &draft.trend_field),
    };
    let saved = if let Some(widget_id) = widget_id {
        crate::boards::update_widget(&state.db, id, widget_id, stored, posted_cell(form)).await
    } else {
        crate::boards::add_widget(&state.db, id, stored).await
    };
    match saved {
        Ok(()) => redirect(&format!("/admin/boards/{id}/edit")),
        Err(Ok(reject)) => {
            let widget_key = widget_id.map(|value| value.to_string());
            board_editor(BoardEdit {
                page: screen(state, headers, registry, locale, user),
                id,
                model_key: Some(model_key),
                widget_id: widget_key.as_deref(),
                alert_key: board_reject_key(reject),
                preview: None,
                form_state: Some(posted_widget(form)),
            })
            .await
        }
        Err(Err(_)) => {
            error_page(
                state,
                headers,
                registry,
                locale,
                StatusCode::INTERNAL_SERVER_ERROR,
                "board_read_error",
                true,
                &user.email,
            )
            .await
        }
    }
}

struct WidgetDraft {
    kind: String,
    fields: Vec<String>,
    group_field: String,
    series_field: String,
    measure: String,
    trend_field: String,
    span: i64,
    title: String,
}

fn widget_draft(
    model: &dyn ManagedModel,
    form: &HashMap<String, String>,
) -> Result<WidgetDraft, &'static str> {
    let kind = form
        .get("kind")
        .map(String::as_str)
        .unwrap_or("")
        .to_owned();
    let fields = selected_fields(model.meta(), form);
    if fields
        .iter()
        .any(|field| field_meta(model.meta(), field).map(audit::concealed) != Some(false))
    {
        return Err("board_bad_model");
    }
    let read = |name: &str| form.get(name).map(String::as_str).unwrap_or("").to_owned();
    let group_field = read("group_field");
    let series_field = read("series_field");
    let measure = read("measure");
    let trend_field = read("trend_field");
    let usable = |name: &str| {
        !name.is_empty() && field_meta(model.meta(), name).map(audit::concealed) == Some(false)
    };
    if matches!(kind.as_str(), "bar" | "line" | "pie" | "tree" | "stack") && !usable(&group_field) {
        return Err("board_group_required");
    }
    if kind == "stack" && (!usable(&series_field) || series_field == group_field) {
        return Err("board_group_required");
    }
    if !measure.is_empty()
        && model
            .columns()
            .iter()
            .find(|column| column.name == measure)
            .is_none_or(|column| column.kind != ColumnKind::BigInt)
    {
        return Err("board_fields_required");
    }
    if kind == "line" && !crate::boards::group_is_date(model.columns(), &group_field) {
        return Err("board_line_date");
    }
    if kind == "card"
        && !trend_field.is_empty()
        && !crate::boards::group_is_date(model.columns(), &trend_field)
    {
        return Err("board_line_date");
    }
    Ok(WidgetDraft {
        kind,
        fields: fields.iter().map(|field| (*field).to_owned()).collect(),
        group_field,
        series_field,
        measure,
        trend_field,
        span: if form.get("span").map(String::as_str) == Some("6") {
            6
        } else {
            12
        },
        title: read("title"),
    })
}

async fn widget_delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    Path((id, widget)): Path<(String, String)>,
    body: Bytes,
) -> Response {
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let user = match posted_superuser(&state, &headers, &registry, &locale, &body).await {
        Ok(user) => user,
        Err(response) => return *response,
    };
    let Some(id) = parse_uuid(&id) else {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    };
    let Some(widget) = parse_uuid(&widget) else {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    };
    if crate::boards::delete_widget(&state.db, id, widget)
        .await
        .is_err()
    {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::INTERNAL_SERVER_ERROR,
            "board_read_error",
            true,
            &user.email,
        )
        .await;
    }
    redirect(&format!("/admin/boards/{id}/edit"))
}

async fn widget_place(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    Path((id, widget)): Path<(String, String)>,
    body: Bytes,
) -> Response {
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let user = match posted_superuser(&state, &headers, &registry, &locale, &body).await {
        Ok(user) => user,
        Err(response) => return *response,
    };
    let fragment = headers.get("x-place").and_then(|value| value.to_str().ok()) == Some("fragment");
    let Some(id) = parse_uuid(&id) else {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    };
    let Some(widget) = parse_uuid(&widget) else {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    };
    let form = parse_form(&body);
    let Some((x, y, w, h)) = posted_cell(&form) else {
        return if fragment {
            plain(StatusCode::BAD_REQUEST, "Request rejected")
        } else {
            redirect(&format!("/admin/boards/{id}/edit"))
        };
    };
    let placed = match crate::boards::place_widget(&state.db, id, widget, x, y, w, h).await {
        Ok(placed) => placed,
        Err(_) => {
            return error_page(
                &state,
                &headers,
                &registry,
                &locale,
                StatusCode::INTERNAL_SERVER_ERROR,
                "board_read_error",
                true,
                &user.email,
            )
            .await;
        }
    };
    if fragment {
        return if placed {
            StatusCode::NO_CONTENT.into_response()
        } else {
            plain(StatusCode::CONFLICT, "occupied")
        };
    }
    redirect(&format!("/admin/boards/{id}/edit"))
}

async fn widget_preview(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let user = match posted_superuser(&state, &headers, &registry, &locale, &body).await {
        Ok(user) => user,
        Err(response) => return *response,
    };
    let fragment = headers
        .get("x-preview")
        .and_then(|value| value.to_str().ok())
        == Some("fragment");
    let Some(id) = parse_uuid(&id) else {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    };
    let form = parse_form(&body);
    let posted = posted_widget(&form);
    let widget_id = form.get("widget").and_then(|value| parse_uuid(value));
    let stored_model = if let Some(widget_id) = widget_id {
        crate::boards::load(&state.db, id)
            .await
            .ok()
            .flatten()
            .and_then(|(_, widgets)| {
                widgets
                    .into_iter()
                    .find(|widget| widget.id == widget_id)
                    .map(|widget| widget.model_key)
            })
    } else {
        None
    };
    let model_key = stored_model.unwrap_or_else(|| form.get("model").cloned().unwrap_or_default());
    let note = |key: &str| preview_note(&locale, key);
    let Some(model) = registry.find(&model_key) else {
        return if fragment {
            note("board_bad_model")
        } else {
            board_editor(BoardEdit {
                page: screen(&state, &headers, &registry, &locale, &user),
                id,
                model_key: Some(model_key.as_str()),
                widget_id: form.get("widget").map(String::as_str),
                alert_key: "board_bad_model",
                preview: None,
                form_state: Some(posted),
            })
            .await
        };
    };
    let draft = match widget_draft(&**model, &form) {
        Ok(draft) => draft,
        Err(key) => {
            return if fragment {
                note(key)
            } else {
                board_editor(BoardEdit {
                    page: screen(&state, &headers, &registry, &locale, &user),
                    id,
                    model_key: Some(model_key.as_str()),
                    widget_id: form.get("widget").map(String::as_str),
                    alert_key: key,
                    preview: None,
                    form_state: Some(posted.clone()),
                })
                .await
            };
        }
    };
    let preview = match crate::boards::materialize(crate::boards::NewWidget {
        kind: &draft.kind,
        model_key: model.meta().key,
        fields: &draft.fields,
        group_field: &draft.group_field,
        series_field: &draft.series_field,
        measure: &draft.measure,
        trend_field: &draft.trend_field,
        span: draft.span,
        title: &draft.title,
        group_is_date: crate::boards::group_is_date(model.columns(), &draft.group_field),
        trend_is_date: crate::boards::group_is_date(model.columns(), &draft.trend_field),
    }) {
        Ok(widget) => widget,
        Err(reject) => {
            let key = board_reject_key(reject);
            return if fragment {
                note(key)
            } else {
                board_editor(BoardEdit {
                    page: screen(&state, &headers, &registry, &locale, &user),
                    id,
                    model_key: Some(model_key.as_str()),
                    widget_id: form.get("widget").map(String::as_str),
                    alert_key: key,
                    preview: None,
                    form_state: Some(posted.clone()),
                })
                .await
            };
        }
    };
    let filters = HashMap::<String, Vec<(&'static str, DbValue)>>::new();
    let block = render_block(&state, &user, &registry, &locale, &preview, &filters).await;
    if fragment {
        return preview_fragment(&locale, &block);
    }
    board_editor(BoardEdit {
        page: screen(&state, &headers, &registry, &locale, &user),
        id,
        model_key: Some(model_key.as_str()),
        widget_id: form.get("widget").map(String::as_str),
        alert_key: "",
        preview: Some(block),
        form_state: Some(posted),
    })
    .await
}

fn posted_widget(form: &HashMap<String, String>) -> Value {
    let fields: Vec<_> = form
        .keys()
        .filter(|key| key.starts_with("field."))
        .map(|key| key.trim_start_matches("field.").to_owned())
        .collect();
    json!({
        "title": form.get("title").cloned().unwrap_or_default(),
        "kind": form.get("kind").cloned().unwrap_or_else(|| "table".to_owned()),
        "fields": fields,
        "group_field": form.get("group_field").cloned().unwrap_or_default(),
        "series_field": form.get("series_field").cloned().unwrap_or_default(),
        "measure": form.get("measure").cloned().unwrap_or_default(),
        "trend_field": form.get("trend_field").cloned().unwrap_or_default(),
        "span": if form.get("span").map(String::as_str) == Some("6") { 6 } else { 12 },
        "x": posted_number(form, "grid_x"),
        "y": posted_number(form, "grid_y"),
        "w": posted_number(form, "grid_w"),
        "h": posted_number(form, "grid_h")
    })
}

fn posted_number(form: &HashMap<String, String>, name: &str) -> i64 {
    form.get(name)
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0)
}

fn posted_cell(form: &HashMap<String, String>) -> Option<(i64, i64, i64, i64)> {
    let read = |name: &str| form.get(name)?.parse::<i64>().ok();
    Some((
        read("grid_x")?,
        read("grid_y")?,
        read("grid_w")?,
        read("grid_h")?,
    ))
}

fn preview_note(locale: &str, key: &str) -> Response {
    let message = text(locale, key)
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    Html(format!(r#"<p class="alert" role="alert">{message}</p>"#)).into_response()
}

fn preview_fragment(locale: &str, block: &Value) -> Response {
    match render(
        "board_block.html",
        json!({ "block": block, "i18n": catalog(locale) }),
        &[],
    ) {
        Ok(body) => Html(body).into_response(),
        Err(error) => {
            tracing::error!(%error, "admin template failed");
            plain(StatusCode::INTERNAL_SERVER_ERROR, "Request rejected")
        }
    }
}

struct BoardEdit<'a> {
    page: PageCtx<'a>,
    id: uuid::Uuid,
    model_key: Option<&'a str>,
    widget_id: Option<&'a str>,
    alert_key: &'a str,
    preview: Option<Value>,
    form_state: Option<Value>,
}

async fn render_board_edit(edit: BoardEdit<'_>) -> Response {
    board_editor(edit).await
}

async fn board_editor(edit: BoardEdit<'_>) -> Response {
    let BoardEdit {
        page,
        id,
        model_key,
        widget_id,
        alert_key,
        preview,
        form_state,
    } = edit;
    let PageCtx {
        state,
        headers,
        registry,
        locale,
        user,
    } = page;
    let loaded = match crate::boards::load(&state.db, id).await {
        Ok(Some(loaded)) => loaded,
        Ok(None) => {
            return error_page(
                state,
                headers,
                registry,
                locale,
                StatusCode::NOT_FOUND,
                "not_found",
                true,
                &user.email,
            )
            .await;
        }
        Err(_) => {
            return error_page(
                state,
                headers,
                registry,
                locale,
                StatusCode::INTERNAL_SERVER_ERROR,
                "board_read_error",
                true,
                &user.email,
            )
            .await;
        }
    };
    let widgets: Vec<_> = loaded
        .1
        .iter()
        .enumerate()
        .map(|(index, widget)| {
            let model_label = registry
                .find(&widget.model_key)
                .map(|model| model.meta().label)
                .unwrap_or(widget.model_key.as_str());
            json!({
                "id": widget.id.to_string(),
                "title": widget.title,
                "kind": widget.kind,
                "kind_label": text(locale, kind_label(&widget.kind)),
                "model_label": model_label,
                "detail": widget_detail(registry, widget),
                "first": index == 0,
                "last": index + 1 == loaded.1.len(),
            })
        })
        .collect();
    let editing = widget_id.and_then(parse_uuid).and_then(|value| {
        loaded
            .1
            .iter()
            .find(|widget| widget.id == value)
            .map(|widget| {
                let model_label = registry
                    .find(&widget.model_key)
                    .map(|model| model.meta().label)
                    .unwrap_or(widget.model_key.as_str());
                json!({
                    "id": widget.id.to_string(),
                    "title": widget.title,
                    "kind": widget.kind,
                    "model_key": widget.model_key,
                    "model_label": model_label,
                    "fields": widget.fields,
                    "group_field": widget.group_field,
                    "series_field": widget.series_field,
                    "measure": widget.measure,
                    "trend_field": widget.trend_field,
                    "span": widget.span,
                    "x": widget.x,
                    "y": widget.y,
                    "w": widget.w,
                    "h": widget.h,
                })
            })
    });
    let models = model_choices(state, user, registry).await;
    let chosen = if editing.is_some() {
        None
    } else {
        model_key.and_then(|key| chosen_model(registry, key))
    };
    let editor = editing
        .as_ref()
        .and_then(|widget| widget["model_key"].as_str())
        .and_then(|key| chosen_model(registry, key))
        .or(chosen);
    let Some(auth) = state.auth.as_ref() else {
        return error_page(
            state,
            headers,
            registry,
            locale,
            StatusCode::FORBIDDEN,
            "forbidden",
            true,
            &user.email,
        )
        .await;
    };
    let csrf = session_csrf(auth, headers).await;
    let alert = if alert_key.is_empty() {
        String::new()
    } else {
        text(locale, alert_key)
    };
    let filters = HashMap::<String, Vec<(&'static str, DbValue)>>::new();
    let layout = render_blocks(state, user, registry, locale, &loaded.1, &filters).await;
    let preview = match preview {
        Some(value) => Some(value),
        None if alert_key.is_empty() => {
            match widget_id
                .and_then(parse_uuid)
                .and_then(|value| loaded.1.iter().find(|item| item.id == value))
            {
                Some(widget) => {
                    Some(render_block(state, user, registry, locale, widget, &filters).await)
                }
                None => None,
            }
        }
        None => None,
    };
    let form_widget = form_state.or_else(|| editing.clone());
    page_as(
        state,
        user,
        registry,
        "board_edit.html",
        locale,
        theme_of(headers),
        &loaded.0.name,
        true,
        user.is_superuser,
        &user.email,
        &csrf,
        &format!("/admin/boards/{id}/edit"),
        "",
        &alert,
        json!({
            "board": { "id": id.to_string(), "name": loaded.0.name, "home": loaded.0.home },
            "widgets": widgets,
            "layout": layout,
            "arranging": true,
            "models": models,
            "chosen": editor,
            "editing": editing,
            "form_widget": form_widget,
            "preview": preview
        }),
    )
    .await
}

struct BoardAlert<'a> {
    page: PageCtx<'a>,
    path: &'a str,
    template: &'a str,
    title: &'a str,
    alert_key: &'a str,
    extra: Value,
}

async fn board_alert(alert: BoardAlert<'_>) -> Response {
    let BoardAlert {
        page,
        path,
        template,
        title,
        alert_key,
        extra,
    } = alert;
    let PageCtx {
        state,
        headers,
        registry,
        locale,
        user,
    } = page;
    let Some(auth) = state.auth.as_ref() else {
        return error_page(
            state,
            headers,
            registry,
            locale,
            StatusCode::FORBIDDEN,
            "forbidden",
            true,
            &user.email,
        )
        .await;
    };
    let csrf = session_csrf(auth, headers).await;
    page_as(
        state,
        user,
        registry,
        template,
        locale,
        theme_of(headers),
        title,
        true,
        user.is_superuser,
        &user.email,
        &csrf,
        path,
        "",
        &text(locale, alert_key),
        extra,
    )
    .await
}

async fn posted_superuser(
    state: &AppState,
    headers: &HeaderMap,
    registry: &AdminRegistry,
    locale: &str,
    body: &Bytes,
) -> Result<AuthUser, Box<Response>> {
    let user = match require_post(state, headers, body).await {
        Ok(user) => user,
        Err(()) => {
            return Err(Box::new(
                error_page(
                    state,
                    headers,
                    registry,
                    locale,
                    StatusCode::FORBIDDEN,
                    "csrf",
                    true,
                    "",
                )
                .await,
            ));
        }
    };
    if !user.is_superuser {
        return Err(Box::new(
            error_page(
                state,
                headers,
                registry,
                locale,
                StatusCode::FORBIDDEN,
                "forbidden",
                true,
                &user.email,
            )
            .await,
        ));
    }
    Ok(user)
}

async fn model_choices(state: &AppState, user: &AuthUser, registry: &AdminRegistry) -> Vec<Value> {
    let Some(auth) = state.auth.as_ref() else {
        return Vec::new();
    };
    let mut models = Vec::new();
    for model in registry.models() {
        if crate::http::list_blocked(model.as_ref()) {
            continue;
        }
        if crate::http::permit(auth, user, model.meta(), "read")
            .await
            .is_err()
        {
            continue;
        }
        let app = registry
            .model_app(model.meta().key)
            .unwrap_or("")
            .to_owned();
        models.push(json!({
            "key": model.meta().key,
            "label": model.meta().label,
            "app": app,
        }));
    }
    models
}

fn chosen_model(registry: &AdminRegistry, key: &str) -> Option<Value> {
    let model = registry.find(key)?;
    let fields: Vec<_> = model
        .meta()
        .fields
        .iter()
        .filter(|field| !audit::concealed(field))
        .map(|field| {
            json!({
                "name": field.name,
                "label": field.label,
                "date": crate::boards::group_is_date(model.columns(), field.name),
                "number": model.columns().iter().any(|column| column.name == field.name && column.kind == ColumnKind::BigInt)
            })
        })
        .collect();
    let app = registry.model_app(key).unwrap_or("").to_owned();
    Some(json!({
        "key": key,
        "label": model.meta().label,
        "app": app,
        "fields": fields
    }))
}

fn selected_fields<'a>(
    meta: &'a fexu_core::AdminModelMeta,
    form: &HashMap<String, String>,
) -> Vec<&'a str> {
    meta.fields
        .iter()
        .filter(|field| {
            form.get(&format!("field.{}", field.name))
                .is_some_and(|value| value == "1")
        })
        .map(|field| field.name)
        .collect()
}

fn field_meta<'a>(
    meta: &'a fexu_core::AdminModelMeta,
    name: &str,
) -> Option<&'a fexu_core::AdminFieldMeta> {
    meta.fields.iter().find(|field| field.name == name)
}

async fn board_slices(
    state: &AppState,
    headers: &HeaderMap,
    user: &AuthUser,
    registry: &AdminRegistry,
    locale: &str,
    widgets: &[crate::boards::Widget],
    params: &HashMap<String, String>,
) -> Result<
    (
        HashMap<String, Vec<(&'static str, DbValue)>>,
        Vec<Value>,
        bool,
    ),
    Box<Response>,
> {
    let mut seen = Vec::new();
    let mut raw_by_model: HashMap<String, String> = HashMap::new();
    let mut sliced = false;
    for widget in widgets {
        let Some(model) = registry.find(&widget.model_key) else {
            continue;
        };
        let mut names: Vec<&str> = widget.fields.iter().map(String::as_str).collect();
        if !widget.group_field.is_empty() {
            names.push(widget.group_field.as_str());
        }
        if !widget.measure.is_empty() {
            names.push(widget.measure.as_str());
        }
        if !widget.series_field.is_empty() {
            names.push(widget.series_field.as_str());
        }
        if !widget.trend_field.is_empty() {
            names.push(widget.trend_field.as_str());
        }
        for name in names {
            let key = (widget.model_key.as_str(), name);
            if seen.contains(&key) {
                continue;
            }
            let Some(field) = field_meta(model.meta(), name) else {
                continue;
            };
            if !field.filterable || audit::concealed(field) {
                continue;
            }
            seen.push(key);
            let value = params
                .get(&format!("slice.{}.{}", widget.model_key, name))
                .map(String::as_str)
                .unwrap_or("");
            if value.is_empty() {
                continue;
            }
            sliced = true;
            let entry = raw_by_model.entry(widget.model_key.clone()).or_default();
            if !entry.is_empty() {
                entry.push('&');
            }
            entry.push_str(&format!("filter.{name}={value}"));
        }
    }
    let mut applied = HashMap::new();
    for (model_key, raw) in &raw_by_model {
        let Some(model) = registry.find(model_key) else {
            continue;
        };
        match crate::http::filters_from(
            raw,
            model.columns(),
            registry,
            model.meta(),
            registry.filters(),
        ) {
            Ok(filters) => {
                applied.insert(model_key.clone(), filters);
            }
            Err(_) => {
                return Err(Box::new(
                    error_page(
                        state,
                        headers,
                        registry,
                        locale,
                        StatusCode::UNPROCESSABLE_ENTITY,
                        "forbidden",
                        true,
                        &user.email,
                    )
                    .await,
                ));
            }
        }
    }
    let mut slicers = Vec::new();
    for (model_key, name) in seen {
        let Some(model) = registry.find(model_key) else {
            continue;
        };
        let Some(field) = field_meta(model.meta(), name) else {
            continue;
        };
        let Some(column) = model.columns().iter().find(|column| column.name == name) else {
            continue;
        };
        let value = params
            .get(&format!("slice.{model_key}.{name}"))
            .cloned()
            .unwrap_or_default();
        let (kind, options) = if column.foreign_table.is_some() {
            let (options, _) = relation_options(registry, state, user, column, &value).await;
            ("uuid", options)
        } else if column.kind == ColumnKind::Boolean {
            (
                "boolean",
                json!([
                    {"id": "true", "label": text(locale, "yes"), "selected": value == "true"},
                    {"id": "false", "label": text(locale, "no"), "selected": value == "false"}
                ]),
            )
        } else if column.kind == ColumnKind::Choice {
            (
                "choice",
                Value::Array(
                    column
                        .choices
                        .iter()
                        .map(|choice| {
                            json!({
                                "id": choice.value,
                                "label": if locale == "de" { choice.label_de } else { choice.label_en },
                                "selected": value == choice.value
                            })
                        })
                        .collect(),
                ),
            )
        } else if column.kind == ColumnKind::BigInt {
            ("number", json!([]))
        } else if column.kind == ColumnKind::Date {
            ("date", json!([]))
        } else {
            ("text", json!([]))
        };
        slicers.push(json!({
            "model": model_key,
            "field": name,
            "label": format!("{} · {}", model.meta().label, field.label),
            "value": value,
            "kind": kind,
            "options": options
        }));
    }
    Ok((applied, slicers, sliced))
}

async fn render_blocks(
    state: &AppState,
    user: &AuthUser,
    registry: &AdminRegistry,
    locale: &str,
    widgets: &[crate::boards::Widget],
    filters: &HashMap<String, Vec<(&'static str, DbValue)>>,
) -> Vec<Value> {
    let mut blocks = Vec::new();
    for widget in widgets {
        blocks.push(render_block(state, user, registry, locale, widget, filters).await);
    }
    blocks
}

async fn render_block(
    state: &AppState,
    user: &AuthUser,
    registry: &AdminRegistry,
    locale: &str,
    widget: &crate::boards::Widget,
    filters: &HashMap<String, Vec<(&'static str, DbValue)>>,
) -> Value {
    stamp_block(
        widget,
        render_block_body(state, user, registry, locale, widget, filters).await,
    )
}

fn stamp_block(widget: &crate::boards::Widget, mut block: Value) -> Value {
    if let Some(object) = block.as_object_mut() {
        object.insert("id".to_owned(), json!(widget.id.to_string()));
        object.insert("x".to_owned(), json!(widget.x));
        object.insert("y".to_owned(), json!(widget.y));
        object.insert("w".to_owned(), json!(widget.w));
        object.insert("h".to_owned(), json!(widget.h));
    }
    block
}

async fn render_block_body(
    state: &AppState,
    user: &AuthUser,
    registry: &AdminRegistry,
    locale: &str,
    widget: &crate::boards::Widget,
    filters: &HashMap<String, Vec<(&'static str, DbValue)>>,
) -> Value {
    let Some(model) = registry.find(&widget.model_key) else {
        return block_error(widget, locale, "board_missing");
    };
    let Some(auth) = state.auth.as_ref() else {
        return block_error(widget, locale, "board_locked");
    };
    if crate::http::permit(auth, user, model.meta(), "read")
        .await
        .is_err()
    {
        return block_error(widget, locale, "board_locked");
    }
    let scope = crate::http::access_scope(model.as_ref(), user, false);
    let model_filters = filters
        .get(&widget.model_key)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    if widget.kind == "table" {
        let fields: Vec<_> = widget
            .fields
            .iter()
            .filter_map(|name| field_meta(model.meta(), name))
            .filter(|field| !audit::concealed(field))
            .collect();
        if fields.is_empty() {
            return block_error(widget, locale, "board_missing");
        }
        let listed = model
            .page(
                &state.db,
                &[],
                None,
                "id",
                false,
                model_filters,
                &[],
                0,
                24,
                &scope,
            )
            .await;
        let Ok((rows, _, _)) = listed else {
            return block_error(widget, locale, "board_read_error");
        };
        let foreign: Vec<ColumnSpec> = model
            .columns()
            .iter()
            .copied()
            .filter(|column| {
                column.foreign_table.is_some()
                    && fields.iter().any(|field| field.name == column.name)
            })
            .collect();
        let relations = if foreign.is_empty() {
            HashMap::new()
        } else {
            match relation_names(registry, state, user, &foreign, &rows).await {
                Ok(relations) => relations,
                Err(_) => return block_error(widget, locale, "board_read_error"),
            }
        };
        let columns: Vec<_> = fields.iter().map(|field| field.label).collect();
        let rendered: Vec<Value> = rows
            .iter()
            .map(|row| {
                let mut lead = false;
                let cells: Vec<String> = fields
                    .iter()
                    .map(|field| {
                        present_cell(
                            locale,
                            registry.branding().currency.as_str(),
                            model.columns(),
                            field.name,
                            row.get(field.name).unwrap_or(&Value::Null),
                            &relations,
                            user.is_superuser,
                            &mut lead,
                        )["text"]
                            .as_str()
                            .unwrap_or("")
                            .to_owned()
                    })
                    .collect();
                let href = row
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|id| Uuid::parse_str(id).is_ok())
                    .map(|id| format!("/admin/models/{}/{}", widget.model_key, id))
                    .unwrap_or_default();
                json!({ "href": href, "cells": cells })
            })
            .collect();
        return json!({
            "title": widget.title,
            "kind": "table",
            "span": 12,
            "model_key": widget.model_key,
            "columns": columns,
            "rows": rendered,
            "error": "",
            "note": ""
        });
    }
    if widget.kind == "card" {
        return render_card(state, user, model.as_ref(), widget, locale, model_filters).await;
    }
    if widget.kind == "stack" {
        return render_stack(
            registry,
            state,
            user,
            model.as_ref(),
            widget,
            locale,
            model_filters,
        )
        .await;
    }
    let group = field_meta(model.meta(), &widget.group_field);
    if group.map(audit::concealed) != Some(false) {
        return block_error(widget, locale, "board_missing");
    }
    let grouped = crate::boards::grouped(
        crate::boards::AggregateQuery {
            db: &state.db,
            table: model.descriptor().table,
            columns: model.columns(),
            scope: &scope,
            filters: model_filters,
        },
        &widget.group_field,
        &widget.measure,
        widget.kind == "line",
    )
    .await;
    let Ok((points, truncated)) = grouped else {
        return block_error(widget, locale, "board_read_error");
    };
    let column = model
        .columns()
        .iter()
        .find(|column| column.name == widget.group_field);
    let mut labeled: Vec<(String, i64)> = points
        .iter()
        .map(|point| (point.label.clone(), point.amount))
        .collect();
    if let Some(column) = column {
        if column.kind == ColumnKind::Choice {
            for (label, _) in &mut labeled {
                if label != "—" {
                    *label = fexu_core::choice_label(column.choices, locale, label);
                }
            }
        } else if column.foreign_table.is_some() {
            let rows: Vec<Value> = labeled
                .iter()
                .filter(|(label, _)| label != "—")
                .map(|(label, _)| json!({ widget.group_field.clone(): label }))
                .collect();
            if let Ok(relations) = relation_names(registry, state, user, &[*column], &rows).await {
                let table = column.foreign_table.unwrap_or("");
                for (label, _) in &mut labeled {
                    if let Some(name) = relations
                        .get(table)
                        .and_then(|entry| entry.labels.get(label.as_str()))
                    {
                        *label = name.clone();
                    }
                }
            }
        }
    }
    if widget.kind == "tree" {
        return tree_block(widget, locale, &labeled, truncated);
    }
    let max = labeled
        .iter()
        .map(|point| point.1)
        .max()
        .unwrap_or(1)
        .max(1);
    let total: i64 = labeled.iter().map(|point| point.1).sum();
    let plot_top = 28.0;
    let plot_bottom = 228.0;
    let plot_left = 48.0;
    let plot_right = 624.0;
    let plot_span = plot_bottom - plot_top;
    let y_of = |amount: i64| plot_bottom - (amount as f64 / max as f64) * plot_span;
    let ticks: Vec<i64> = if max <= 5 {
        (0..=max).collect()
    } else {
        vec![0, max / 2, max]
    };
    let grid: Vec<Value> = ticks
        .iter()
        .map(|tick| {
            json!({
                "y": format!("{:.1}", y_of(*tick)),
                "label": tick.to_string()
            })
        })
        .collect();
    let slot = if labeled.is_empty() {
        1.0
    } else {
        (plot_right - plot_left) / labeled.len() as f64
    };
    let mut slices = Vec::new();
    let mut bars = Vec::new();
    let mut dots = Vec::new();
    let mut point_list = String::new();
    let mut cursor = 0.0_f64;
    let ring = 2.0 * std::f64::consts::PI * 70.0;
    for (index, (label, amount)) in labeled.iter().enumerate() {
        let color = CHART_COLORS[index % CHART_COLORS.len()];
        let share = if total == 0 {
            0.0
        } else {
            *amount as f64 / total as f64 * 100.0
        };
        let bar_w = (slot * 0.62).clamp(8.0, 72.0);
        let bar_x = plot_left + slot * index as f64 + (slot - bar_w) / 2.0;
        let bar_y = y_of(*amount);
        let bar_h = (plot_bottom - bar_y).max(if *amount > 0 { 2.0 } else { 0.0 });
        let center = bar_x + bar_w / 2.0;
        let x = if labeled.len() <= 1 {
            (plot_left + plot_right) / 2.0
        } else {
            plot_left + index as f64 * (plot_right - plot_left) / (labeled.len() - 1) as f64
        };
        let y = y_of(*amount);
        if !point_list.is_empty() {
            point_list.push(' ');
        }
        point_list.push_str(&format!("{x:.1},{y:.1}"));
        let dash = share / 100.0 * ring;
        slices.push(json!({
            "label": label,
            "amount": amount,
            "share": format!("{share:.0}"),
            "color": color,
            "dash": format!("{dash:.2} {:.2}", ring - dash),
            "offset": format!("{:.2}", -cursor / 100.0 * ring)
        }));
        bars.push(json!({
            "x": format!("{bar_x:.1}"),
            "y": format!("{bar_y:.1}"),
            "w": format!("{bar_w:.1}"),
            "h": format!("{bar_h:.1}"),
            "cx": format!("{center:.1}"),
            "value_y": format!("{:.1}", (bar_y - 8.0).max(14.0)),
            "label": clip_chart_label(label, 24),
            "amount": amount,
            "color": color
        }));
        dots.push(json!({ "x": format!("{x:.1}"), "y": format!("{y:.1}"), "amount": amount }));
        cursor += share;
    }
    let area = if labeled.is_empty() {
        String::new()
    } else {
        let first_x = dots
            .first()
            .and_then(|dot| dot["x"].as_str())
            .unwrap_or("48");
        let last_x = dots
            .last()
            .and_then(|dot| dot["x"].as_str())
            .unwrap_or("624");
        format!("{first_x},{plot_bottom:.1} {point_list} {last_x},{plot_bottom:.1}")
    };
    let note = if truncated {
        text(
            locale,
            if widget.kind == "line" {
                "board_line_limit"
            } else {
                "board_chart_limit"
            },
        )
    } else {
        String::new()
    };
    json!({
        "title": widget.title,
        "kind": widget.kind,
        "span": if widget.span == 6 { 6 } else { 12 },
        "model_key": widget.model_key,
        "slices": slices,
        "bars": bars,
        "grid": grid,
        "points": point_list,
        "area": area,
        "dots": dots,
        "tilt": labeled.iter().any(|(label, _)| label.chars().count() > 12),
        "total": total,
        "first": labeled.first().map(|point| point.0.clone()).unwrap_or_default(),
        "last": labeled.last().map(|point| point.0.clone()).unwrap_or_default(),
        "error": "",
        "note": note
    })
}

async fn render_card(
    state: &AppState,
    user: &AuthUser,
    model: &dyn ManagedModel,
    widget: &crate::boards::Widget,
    locale: &str,
    filters: &[(&str, DbValue)],
) -> Value {
    let scope = crate::http::access_scope(model, user, false);
    let total = match crate::boards::scalar(
        &state.db,
        model.descriptor().table,
        model.columns(),
        &scope,
        &widget.measure,
        filters,
    )
    .await
    {
        Ok(total) => total,
        Err(_) => return block_error(widget, locale, "board_read_error"),
    };
    let mut spark = String::new();
    let mut delta = String::new();
    let mut direction = "";
    if !widget.trend_field.is_empty() {
        let grouped = crate::boards::grouped(
            crate::boards::AggregateQuery {
                db: &state.db,
                table: model.descriptor().table,
                columns: model.columns(),
                scope: &scope,
                filters,
            },
            &widget.trend_field,
            &widget.measure,
            true,
        )
        .await;
        let Ok((points, _)) = grouped else {
            return block_error(widget, locale, "board_read_error");
        };
        if points.len() >= 2 {
            let previous = points[points.len() - 2].amount;
            let latest = points[points.len() - 1].amount;
            let diff = latest - previous;
            direction = if diff > 0 {
                "up"
            } else if diff < 0 {
                "down"
            } else {
                "flat"
            };
            delta = if previous != 0 {
                let percent = (diff as f64 / previous as f64 * 100.0).round() as i64;
                format!("{diff:+} · {percent:+}%")
            } else {
                format!("{diff:+}")
            };
        }
        let max = points
            .iter()
            .map(|point| point.amount)
            .max()
            .unwrap_or(1)
            .max(1);
        for (index, point) in points.iter().enumerate() {
            let x = if points.len() <= 1 {
                60.0
            } else {
                index as f64 * 120.0 / (points.len() - 1) as f64
            };
            let y = 32.0 - point.amount as f64 / max as f64 * 28.0;
            if !spark.is_empty() {
                spark.push(' ');
            }
            spark.push_str(&format!("{x:.1},{y:.1}"));
        }
    }
    json!({
        "title": widget.title,
        "kind": "card",
        "span": 4,
        "model_key": widget.model_key,
        "model_label": model.meta().label,
        "total": total,
        "delta": delta,
        "direction": direction,
        "spark": spark,
        "error": "",
        "note": ""
    })
}

async fn render_stack(
    registry: &AdminRegistry,
    state: &AppState,
    user: &AuthUser,
    model: &dyn ManagedModel,
    widget: &crate::boards::Widget,
    locale: &str,
    filters: &[(&str, DbValue)],
) -> Value {
    if field_meta(model.meta(), &widget.group_field).map(audit::concealed) != Some(false)
        || field_meta(model.meta(), &widget.series_field).map(audit::concealed) != Some(false)
    {
        return block_error(widget, locale, "board_missing");
    }
    let scope = crate::http::access_scope(model, user, false);
    let stacked = crate::boards::stacked(
        crate::boards::AggregateQuery {
            db: &state.db,
            table: model.descriptor().table,
            columns: model.columns(),
            scope: &scope,
            filters,
        },
        &widget.group_field,
        &widget.series_field,
        &widget.measure,
    )
    .await;
    let Ok(points) = stacked else {
        return block_error(widget, locale, "board_read_error");
    };
    let mut group_raw = Vec::new();
    let mut series_raw = Vec::new();
    for point in &points {
        if !group_raw.contains(&point.group) {
            group_raw.push(point.group.clone());
        }
        if !series_raw.contains(&point.series) {
            series_raw.push(point.series.clone());
        }
    }
    let mut group_names = group_raw.clone();
    let mut series_names = series_raw.clone();
    name_axis(
        registry,
        state,
        user,
        locale,
        model,
        &widget.group_field,
        &mut group_names,
    )
    .await;
    name_axis(
        registry,
        state,
        user,
        locale,
        model,
        &widget.series_field,
        &mut series_names,
    )
    .await;
    let group_map: HashMap<_, _> = group_raw.into_iter().zip(group_names).collect();
    let series_map: HashMap<_, _> = series_raw.into_iter().zip(series_names).collect();
    let named: Vec<(String, String, i64)> = points
        .into_iter()
        .map(|point| {
            (
                group_map.get(&point.group).cloned().unwrap_or(point.group),
                series_map
                    .get(&point.series)
                    .cloned()
                    .unwrap_or(point.series),
                point.amount,
            )
        })
        .collect();
    let mut group_order: Vec<(String, i64)> = Vec::new();
    let mut series_order: Vec<(String, i64)> = Vec::new();
    for (group, series_name, amount) in &named {
        if let Some(found) = group_order.iter_mut().find(|item| item.0 == *group) {
            found.1 += amount;
        } else {
            group_order.push((group.clone(), *amount));
        }
        if let Some(found) = series_order.iter_mut().find(|item| item.0 == *series_name) {
            found.1 += amount;
        } else {
            series_order.push((series_name.clone(), *amount));
        }
    }
    group_order.sort_by_key(|item| std::cmp::Reverse(item.1));
    series_order.sort_by_key(|item| std::cmp::Reverse(item.1));
    let truncated = group_order.len() > 12 || series_order.len() > 6;
    group_order.truncate(12);
    series_order.truncate(6);
    let max = group_order
        .iter()
        .map(|item| item.1)
        .max()
        .unwrap_or(1)
        .max(1);
    let legend: Vec<_> = series_order
        .iter()
        .enumerate()
        .map(|(index, (label, _))| {
            json!({ "label": label, "color": CHART_COLORS[index % CHART_COLORS.len()] })
        })
        .collect();
    let mut rows = Vec::new();
    for (index, (group, total)) in group_order.iter().enumerate() {
        let mut cursor = 150.0;
        let mut segments = Vec::new();
        for (series_index, (series_name, _)) in series_order.iter().enumerate() {
            let amount = named
                .iter()
                .find(|item| item.0 == *group && item.1 == *series_name)
                .map(|item| item.2)
                .unwrap_or(0);
            if amount <= 0 {
                continue;
            }
            let width = amount as f64 / max as f64 * 450.0;
            segments.push(json!({
                "x": format!("{cursor:.1}"),
                "w": format!("{width:.1}"),
                "amount": amount,
                "color": CHART_COLORS[series_index % CHART_COLORS.len()]
            }));
            cursor += width;
        }
        rows.push(json!({
            "label": clip_chart_label(group, 18),
            "y": 24 + index * 28,
            "total": total,
            "segments": segments
        }));
    }
    let height = 36 + group_order.len() * 28 + 28;
    let ticks = if max <= 5 {
        vec![0, max]
    } else {
        vec![0, max / 2, max]
    };
    let axis: Vec<_> = ticks
        .iter()
        .map(|tick| {
            json!({
                "x": format!("{:.1}", 150.0 + *tick as f64 / max as f64 * 450.0),
                "label": tick.to_string()
            })
        })
        .collect();
    json!({
        "title": widget.title,
        "kind": "stack",
        "span": if widget.span == 6 { 6 } else { 12 },
        "model_key": widget.model_key,
        "rows": rows,
        "legend": legend,
        "axis": axis,
        "height": height,
        "error": "",
        "note": if truncated { text(locale, "board_chart_limit") } else { String::new() }
    })
}

async fn name_axis(
    registry: &AdminRegistry,
    state: &AppState,
    user: &AuthUser,
    locale: &str,
    model: &dyn ManagedModel,
    field: &str,
    labels: &mut [String],
) {
    let Some(column) = model.columns().iter().find(|column| column.name == field) else {
        return;
    };
    if column.kind == ColumnKind::Choice {
        for label in labels.iter_mut() {
            if label != "—" {
                *label = fexu_core::choice_label(column.choices, locale, label);
            }
        }
        return;
    }
    let Some(table) = column.foreign_table else {
        return;
    };
    let rows: Vec<Value> = labels
        .iter()
        .filter(|label| label.as_str() != "—")
        .map(|label| json!({ field: label }))
        .collect();
    if let Ok(relations) = relation_names(registry, state, user, &[*column], &rows).await {
        for label in labels.iter_mut() {
            if let Some(name) = relations
                .get(table)
                .and_then(|entry| entry.labels.get(label.as_str()))
            {
                *label = name.clone();
            }
        }
    }
}

fn tree_block(
    widget: &crate::boards::Widget,
    locale: &str,
    labeled: &[(String, i64)],
    truncated: bool,
) -> Value {
    let mut tiles = Vec::new();
    let positive: Vec<_> = labeled
        .iter()
        .filter(|(_, amount)| *amount > 0)
        .enumerate()
        .map(|(index, (label, amount))| {
            (
                label.clone(),
                *amount,
                CHART_COLORS[index % CHART_COLORS.len()],
            )
        })
        .collect();
    layout_tiles(&positive, 8.0, 8.0, 624.0, 304.0, &mut tiles);
    json!({
        "title": widget.title,
        "kind": "tree",
        "span": if widget.span == 6 { 6 } else { 12 },
        "model_key": widget.model_key,
        "tiles": tiles,
        "error": "",
        "note": if truncated { text(locale, "board_chart_limit") } else { String::new() }
    })
}

fn layout_tiles(
    items: &[(String, i64, &str)],
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    tiles: &mut Vec<Value>,
) {
    if items.is_empty() || width < 1.0 || height < 1.0 {
        return;
    }
    let total: f64 = items.iter().map(|item| item.1 as f64).sum();
    if total <= 0.0 {
        return;
    }
    if items.len() == 1 {
        tiles.push(json!({
            "x": format!("{x:.1}"),
            "y": format!("{y:.1}"),
            "w": format!("{width:.1}"),
            "h": format!("{height:.1}"),
            "color": items[0].2,
            "label": clip_chart_label(&items[0].0, 22),
            "full": items[0].0,
            "amount": items[0].1,
            "lx": format!("{:.1}", x + 10.0),
            "ly": format!("{:.1}", y + 22.0),
            "vy": format!("{:.1}", y + 40.0),
            "text": width >= 72.0 && height >= 36.0
        }));
        return;
    }
    let mut split = 1;
    let mut consumed = 0.0;
    for (index, item) in items.iter().enumerate() {
        consumed += item.1 as f64;
        split = index + 1;
        if consumed >= total / 2.0 {
            break;
        }
    }
    split = split.clamp(1, items.len() - 1);
    let (left, right) = items.split_at(split);
    let left_sum: f64 = left.iter().map(|item| item.1 as f64).sum();
    let share = left_sum / total;
    if width >= height {
        let left_width = (width * share).clamp(1.0, width - 1.0);
        layout_tiles(left, x, y, left_width, height, tiles);
        layout_tiles(right, x + left_width, y, width - left_width, height, tiles);
    } else {
        let left_height = (height * share).clamp(1.0, height - 1.0);
        layout_tiles(left, x, y, width, left_height, tiles);
        layout_tiles(
            right,
            x,
            y + left_height,
            width,
            height - left_height,
            tiles,
        );
    }
}

fn widget_detail(registry: &AdminRegistry, widget: &crate::boards::Widget) -> String {
    let Some(model) = registry.find(&widget.model_key) else {
        return String::new();
    };
    let label = |name: &str| -> String {
        field_meta(model.meta(), name)
            .map(|field| field.label.to_string())
            .unwrap_or_else(|| name.to_string())
    };
    if widget.kind == "table" {
        return widget
            .fields
            .iter()
            .map(|name| label(name))
            .collect::<Vec<_>>()
            .join(", ");
    }
    let mut parts = Vec::new();
    for name in [
        widget.group_field.as_str(),
        widget.series_field.as_str(),
        widget.measure.as_str(),
        widget.trend_field.as_str(),
    ] {
        if !name.is_empty() {
            parts.push(label(name));
        }
    }
    parts.join(" · ")
}

fn clip_chart_label(label: &str, limit: usize) -> String {
    let chars: Vec<char> = label.chars().collect();
    if chars.len() <= limit {
        label.to_owned()
    } else {
        format!("{}…", chars.into_iter().take(limit - 1).collect::<String>())
    }
}

fn block_error(widget: &crate::boards::Widget, locale: &str, key: &str) -> Value {
    json!({
        "title": widget.title,
        "kind": widget.kind,
        "span": 12,
        "model_key": widget.model_key,
        "error": text(locale, key),
        "note": "",
        "rows": [],
        "columns": [],
        "slices": []
    })
}

fn kind_label(kind: &str) -> &'static str {
    match kind {
        "table" => "board_table",
        "bar" => "board_bar",
        "line" => "board_line",
        "pie" => "board_pie",
        "stack" => "board_stack",
        "tree" => "board_tree",
        "card" => "board_card",
        _ => "board_kind",
    }
}

fn board_reject_key(reject: crate::boards::BoardReject) -> &'static str {
    match reject {
        crate::boards::BoardReject::Name => "board_name_invalid",
        crate::boards::BoardReject::Title => "board_title_invalid",
        crate::boards::BoardReject::Limit => "board_limit",
        crate::boards::BoardReject::WidgetLimit => "board_widget_limit",
        crate::boards::BoardReject::Fields => "board_fields_required",
        crate::boards::BoardReject::Group => "board_group_required",
        crate::boards::BoardReject::LineDate => "board_line_date",
        crate::boards::BoardReject::Kind => "board_bad_kind",
        crate::boards::BoardReject::Model => "board_bad_model",
        crate::boards::BoardReject::Place => "board_place_busy",
    }
}

fn text(locale: &str, key: &str) -> String {
    catalog(locale)
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or(key)
        .to_owned()
}

fn parse_uuid(value: &str) -> Option<uuid::Uuid> {
    if is_uuid(value) {
        uuid::Uuid::parse_str(value).ok()
    } else {
        None
    }
}

const CHART_COLORS: &[&str] = &[
    "#6d28d9", "#0f766e", "#b45309", "#1d4ed8", "#9f1239", "#0369a1", "#7c3aed", "#15803d",
    "#c2410c", "#4338ca", "#0e7490", "#a16207",
];

async fn model_index(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
) -> Response {
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let user = match require_get(&state, &headers, &registry, "/admin/models").await {
        Ok(user) => user,
        Err(response) => return response,
    };
    let Some(auth) = state.auth.as_ref() else {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::FORBIDDEN,
            "forbidden",
            true,
            &user.email,
        )
        .await;
    };
    let csrf = session_csrf(auth, &headers).await;
    page_as(
        &state,
        &user,
        &registry,
        "dashboard.html",
        &locale,
        theme_of(&headers),
        catalog(&locale)["models"].as_str().unwrap_or("Models"),
        true,
        user.is_superuser,
        &user.email,
        &csrf,
        "/admin/models",
        "",
        "",
        overview_extra(&state, &user, &locale).await,
    )
    .await
}

async fn model_list(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    Path(model_key): Path<String>,
    Query(params): Query<HashMap<String, String>>,
    RawQuery(raw): RawQuery,
) -> Response {
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let user = match require_get(
        &state,
        &headers,
        &registry,
        &format!("/admin/models/{model_key}"),
    )
    .await
    {
        Ok(user) => user,
        Err(response) => return response,
    };
    let Some(model) = registry.find(&model_key) else {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    };
    if gate(&state, &registry, &user, model.as_ref(), "read")
        .await
        .is_err()
        || list_blocked(model.as_ref())
    {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::FORBIDDEN,
            "forbidden",
            true,
            &user.email,
        )
        .await;
    }
    let page_number = params
        .get("page")
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(1)
        .max(1);
    let (sort, descending) = match resolved_sort(
        registry.field_slice(model.meta()),
        registry.ordering_of(model.meta()),
        &params,
    ) {
        Ok(value) => value,
        Err(()) => {
            return error_page(
                &state,
                &headers,
                &registry,
                &locale,
                StatusCode::UNPROCESSABLE_ENTITY,
                "forbidden",
                true,
                &user.email,
            )
            .await;
        }
    };
    let filters = match filters_from(
        raw.as_deref().unwrap_or(""),
        model.columns(),
        &registry,
        model.meta(),
        registry.filters(),
    ) {
        Ok(filters) => filters,
        Err(_) => {
            return error_page(
                &state,
                &headers,
                &registry,
                &locale,
                StatusCode::UNPROCESSABLE_ENTITY,
                "forbidden",
                true,
                &user.email,
            )
            .await
        }
    };
    let filter_refs: Vec<(&str, DbValue)> = filters
        .iter()
        .map(|(name, value)| (*name, value.clone()))
        .collect();
    let search_columns: Vec<&str> = registry
        .field_slice(model.meta())
        .iter()
        .filter(|field| field.searchable)
        .map(|field| field.name)
        .collect();
    let query = params.get("q").cloned().unwrap_or_default();
    if !query.is_empty() && search_columns.is_empty() {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::UNPROCESSABLE_ENTITY,
            "forbidden",
            true,
            &user.email,
        )
        .await;
    }
    let scope = access_scope(model.as_ref(), &user, false);
    let navigation = match date_navigation(
        &state,
        model.as_ref(),
        &locale,
        &params,
        &search_columns,
        (!query.is_empty()).then_some(query.as_str()),
        &filter_refs,
        &scope,
        &format!("/admin/models/{model_key}"),
    )
    .await
    {
        Ok(value) => value,
        Err(()) => {
            return error_page(
                &state,
                &headers,
                &registry,
                &locale,
                StatusCode::UNPROCESSABLE_ENTITY,
                "forbidden",
                true,
                &user.email,
            )
            .await;
        }
    };
    let page_size = u64::from(registry.per_page_of(model.meta()).clamp(1, 100));
    let listed = model
        .page(
            &state.db,
            &search_columns,
            (!query.is_empty()).then_some(query.as_str()),
            &sort,
            descending,
            &filter_refs,
            &navigation.bounds,
            u64::from(page_number - 1) * page_size,
            page_size,
            &scope,
        )
        .await;
    let (mut rows, has_more, total) = match listed {
        Ok(value) => value,
        Err(_) => {
            return error_page(
                &state,
                &headers,
                &registry,
                &locale,
                StatusCode::INTERNAL_SERVER_ERROR,
                "forbidden",
                true,
                &user.email,
            )
            .await
        }
    };
    for row in &mut rows {
        redact(model.meta(), row);
    }
    let columns = list_columns(
        registry.field_slice(model.meta()),
        model.columns(),
        &model_key,
        &params,
        &sort,
        descending,
        registry.list_order_of(model.meta()),
    );
    let links = link_names(
        registry.field_slice(model.meta()),
        model.columns(),
        registry.list_order_of(model.meta()),
    );
    let relations = match relation_names(&registry, &state, &user, model.columns(), &rows).await {
        Ok(relations) => relations,
        Err(_) => {
            return error_page(
                &state,
                &headers,
                &registry,
                &locale,
                StatusCode::INTERNAL_SERVER_ERROR,
                "forbidden",
                true,
                &user.email,
            )
            .await
        }
    };
    let image_fields: Vec<&str> = registry
        .widgets()
        .iter()
        .filter(|widget| {
            widget.model_key == model_key
                && matches!(widget.kind, crate::widgets::WidgetKind::Image)
        })
        .map(|widget| widget.field)
        .collect();
    let row_ids: Vec<Uuid> = rows
        .iter()
        .filter_map(|row| {
            row.get("id")
                .and_then(Value::as_str)
                .and_then(|value| Uuid::parse_str(value).ok())
        })
        .collect();
    let mut images = HashMap::new();
    for field in &image_fields {
        if let Ok(found) = crate::files::stored_ids(&state.db, &model_key, field, &row_ids).await {
            images.insert(*field, found);
        }
    }
    let list_edit: Vec<&str> =
        if permit(state.auth.as_ref().unwrap(), &user, model.meta(), "update")
            .await
            .is_ok()
        {
            registry.list_editable_of(model.meta()).to_vec()
        } else {
            Vec::new()
        };
    let rendered_rows: Vec<Value> = rows
        .iter()
        .map(|row| {
            let mut lead_taken = false;
            let cells: Vec<Value> = columns
                .iter()
                .map(|column| {
                    let name = column["name"].as_str().unwrap_or("");
                    let mut cell = present_cell(
                        &locale,
                        registry.branding().currency.as_str(),
                        model.columns(),
                        name,
                        row.get(name).unwrap_or(&Value::Null),
                        &relations,
                        user.is_superuser,
                        &mut lead_taken,
                    );
                    if links.contains(&name) {
                        cell["link"] = json!(true);
                    }
                    cell["name"] = json!(name);
                    if list_edit.contains(&name) {
                        if let Some(column) =
                            model.columns().iter().find(|column| column.name == name)
                        {
                            cell["edit"] =
                                list_editor(&locale, column, row.get(name).unwrap_or(&Value::Null));
                        }
                    }
                    if let (Some(found), Some(id)) = (
                        images.get(name),
                        row.get("id")
                            .and_then(Value::as_str)
                            .and_then(|value| Uuid::parse_str(value).ok()),
                    ) {
                        if found.contains(&id) {
                            cell["image"] =
                                json!(format!("/admin/models/{model_key}/{id}/file/{name}"));
                        }
                    }
                    cell
                })
                .collect();
            json!({
            "id": row.get("id").and_then(Value::as_str).unwrap_or(""),
                    "cells": cells
                })
        })
        .collect();
    let csrf = session_csrf(state.auth.as_ref().unwrap(), &headers).await;
    let list_path = format!("/admin/models/{model_key}");
    let preserved = preserve_query(&params, page_number);
    let filters = filter_inputs(
        &locale,
        &registry,
        &state,
        &user,
        model.meta(),
        model.columns(),
        &params,
    )
    .await;
    let filtering = !query.is_empty()
        || filters.iter().any(|item| {
            item.get("value")
                .and_then(Value::as_str)
                .is_some_and(|value| !value.is_empty())
        });
    let can_update = permit(state.auth.as_ref().unwrap(), &user, model.meta(), "update")
        .await
        .is_ok();
    let can_delete = permit(state.auth.as_ref().unwrap(), &user, model.meta(), "delete")
        .await
        .is_ok();
    let mut actions = Vec::new();
    for action in registry.actions_for(model.meta(), model.columns()) {
        if permit(
            state.auth.as_ref().unwrap(),
            &user,
            model.meta(),
            action.permission,
        )
        .await
        .is_ok()
        {
            actions.push(json!({
                "name": action.name,
                "label": if locale == "de" { action.label_de } else { action.label_en }
            }));
        }
    }
    let changed = params
        .get("changed")
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|count| *count <= 100);
    let notice = if params.get("notice").map(String::as_str) == Some("changed") {
        changed
            .map(|count| bulk_notice(&locale, count))
            .unwrap_or_default()
    } else {
        flash(
            &locale,
            params.get("notice").map(String::as_str).unwrap_or(""),
        )
    };
    let typed_keys: Vec<String> = filters
        .iter()
        .filter(|item| item.get("facet").and_then(Value::as_bool) != Some(true))
        .filter_map(|item| item.get("name").and_then(Value::as_str))
        .map(|name| format!("filter.{name}"))
        .collect();
    let mut kept_pairs: Vec<_> = params
        .iter()
        .filter(|(key, value)| {
            !value.is_empty() && !matches!(key.as_str(), "page" | "notice" | "changed")
        })
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    kept_pairs.sort();
    let kept: Vec<Value> = kept_pairs
        .into_iter()
        .map(|(name, value)| {
            json!({
                "name": name,
                "value": value,
                "typed": typed_keys.iter().any(|key| key == &name)
            })
        })
        .collect();
    let has_value_filters = filters
        .iter()
        .any(|item| item.get("facet").and_then(Value::as_bool) != Some(true));
    page_as(
        &state,
        &user,
        &registry,
        "list.html",
        &locale,
        theme_of(&headers),
        model.meta().label,
        true,
        user.is_superuser,
        &user.email,
        &csrf,
        &list_path,
        &notice,
        "",
        json!({
            "model_key": model.meta().key,
            "model_label": model.meta().label,
            "actions": actions,
            "list_path": list_path,
            "query": query,
            "sort": if params.contains_key("sort") { sort.clone() } else { String::new() },
            "dir": if params.contains_key("dir") { if descending { "desc" } else { "asc" }.to_owned() } else { String::new() },
            "filters": filters,
            "has_value_filters": has_value_filters,
            "kept": kept,
            "filtering": filtering,
            "hierarchy": navigation.view,
            "can_create": permit(state.auth.as_ref().unwrap(), &user, model.meta(), "create").await.is_ok(),
            "can_update": can_update,
            "can_delete": can_delete,
            "columns": columns,
            "pictures": !image_fields.is_empty(),
            "editable": !list_edit.is_empty(),
            "rows": rendered_rows,
            "total": total,
            "shown": rows.len(),
            "page": page_number,
            "previous": (page_number > 1).then(|| format!("{list_path}?{preserved}&page={}", page_number - 1)),
            "next_page": has_more.then(|| format!("{list_path}?{preserved}&page={}", page_number + 1))
        }),
    )
    .await
}

async fn model_new(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    Path(model_key): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let mut values = Map::new();
    if params.contains_key("draft") {
        values.insert("_draft".to_owned(), json!(params.get("draft")));
        values.insert("_use_draft".to_owned(), json!(true));
    }
    if params.get("popup").is_some_and(|value| value == "1") {
        values.insert("_popup".to_owned(), json!(true));
    }
    if let Some(model) = registry.find(&model_key) {
        let mut picks = Map::new();
        apply_picks(model.meta(), model.columns(), &params, &mut picks);
        if !picks.is_empty() {
            values.insert("_picks".to_owned(), Value::Object(picks));
        }
    }
    let notice = flash(
        &locale_of(
            &headers,
            &registry.branding().default_locale,
            &registry.branding().fallback_locale,
        ),
        params.get("notice").map(String::as_str).unwrap_or(""),
    );
    form_page(
        &state,
        &headers,
        &registry,
        &model_key,
        None,
        values,
        FormFeedback {
            notice: &notice,
            alert: "",
        },
    )
    .await
}

async fn model_create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    Path(model_key): Path<String>,
    body: Bytes,
) -> Response {
    save_form(&state, &headers, &registry, &model_key, None, &body).await
}

async fn model_detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    Path((model_key, id)): Path<(String, Uuid)>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let user = match require_get(
        &state,
        &headers,
        &registry,
        &format!("/admin/models/{model_key}/{id}"),
    )
    .await
    {
        Ok(user) => user,
        Err(response) => return response,
    };
    let Some(model) = registry.find(&model_key) else {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    };
    if gate(&state, &registry, &user, model.as_ref(), "read")
        .await
        .is_err()
    {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    }
    let Some(mut record) = read_visible(&state, &user, model.as_ref(), id, "read").await else {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    };
    let can_update = gate(&state, &registry, &user, model.as_ref(), "update")
        .await
        .is_ok()
        && read_visible(&state, &user, model.as_ref(), id, "update")
            .await
            .is_some();
    if query.contains_key("view") {
        return redirect(&format!("/admin/models/{model_key}/{id}"));
    }
    if can_update {
        return record_form(
            State(state),
            headers,
            Extension(registry),
            Path((model_key, id)),
            Query(query),
        )
        .await;
    }
    let can_delete = gate(&state, &registry, &user, model.as_ref(), "delete")
        .await
        .is_ok()
        && read_visible(&state, &user, model.as_ref(), id, "delete")
            .await
            .is_some();
    redact(model.meta(), &mut record);
    let relations = relation_names(&registry, &state, &user, model.columns(), &[record.clone()])
        .await
        .unwrap_or_default();
    let fields = visible_fields(
        &locale,
        registry.branding().currency.as_str(),
        model.meta(),
        model.columns(),
        &record,
        &relations,
        user.is_superuser,
    );
    let csrf = session_csrf(state.auth.as_ref().unwrap(), &headers).await;
    let list_path = format!("/admin/models/{model_key}");
    let history = record_history(&state, &user, &locale, model.meta().key, &id.to_string()).await;
    page_as(
        &state,
        &user,
        &registry,
        "detail.html",
        &locale,
        theme_of(&headers),
        model.meta().label,
        true,
        user.is_superuser,
        &user.email,
        &csrf,
        &list_path,
        &flash(
            &locale,
            query.get("notice").map(String::as_str).unwrap_or(""),
        ),
        "",
        json!({
            "model_label": model.meta().label,
            "list_path": list_path,
            "id": id.to_string(),
            "history_href": history_href(model.meta().key, &id.to_string()),
            "history": history.events,
            "history_more": history.more,
            "fields": fields,
            "can_update": false,
            "can_delete": can_delete,
            "popup": query.get("popup").is_some_and(|value| value == "1"),
            "nav": !query.get("popup").is_some_and(|value| value == "1"),
            "inlines": inline_sections(&state, &registry, &user, model.as_ref(), id, &query, false, &locale).await
        }),
    )
    .await
}

async fn record_form(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    Path((model_key, id)): Path<(String, Uuid)>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    if let Err(response) = require_get(
        &state,
        &headers,
        &registry,
        &format!("/admin/models/{model_key}/{id}"),
    )
    .await
    {
        return response;
    }
    let Some(mut record) = load_for_form(&state, &headers, &registry, &model_key, id).await else {
        let locale = locale_of(
            &headers,
            &registry.branding().default_locale,
            &registry.branding().fallback_locale,
        );
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            "",
        )
        .await;
    };
    record.insert("_inline_query".to_owned(), json!(params));
    if params.get("popup").is_some_and(|value| value == "1") {
        record.insert("_popup".to_owned(), json!(true));
    }
    if params.contains_key("draft") {
        record.insert(
            "_draft".to_owned(),
            json!(params.get("draft").map(String::as_str).unwrap_or("")),
        );
        record.insert("_use_draft".to_owned(), json!(true));
    }
    if let Some(model) = registry.find(&model_key) {
        let mut picks = Map::new();
        apply_picks(model.meta(), model.columns(), &params, &mut picks);
        if !picks.is_empty() {
            record.insert("_picks".to_owned(), Value::Object(picks));
        }
    }
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let notice = flash(
        &locale,
        params.get("notice").map(String::as_str).unwrap_or(""),
    );
    form_page(
        &state,
        &headers,
        &registry,
        &model_key,
        Some(id),
        record,
        FormFeedback {
            notice: &notice,
            alert: "",
        },
    )
    .await
}

fn list_editor(locale: &str, column: &ColumnSpec, value: &Value) -> Value {
    let text = match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        Value::Bool(flag) => {
            if *flag {
                "true".to_owned()
            } else {
                "false".to_owned()
            }
        }
        _ => String::new(),
    };
    let input = match column.kind {
        ColumnKind::Boolean => "checkbox",
        ColumnKind::BigInt => "number",
        ColumnKind::Date => "date",
        ColumnKind::Time => "time",
        ColumnKind::DateTime => "datetime-local",
        ColumnKind::Choice => "select",
        _ => "text",
    };
    let shown = if column.kind == ColumnKind::DateTime {
        text.replace(' ', "T")
    } else {
        text
    };
    let options: Vec<Value> = column
        .choices
        .iter()
        .map(|choice| {
            json!({
                "value": choice.value,
                "label": if locale == "de" { choice.label_de } else { choice.label_en },
                "selected": choice.value == shown
            })
        })
        .collect();
    json!({
        "input": input,
        "value": shown,
        "checked": shown == "true" || shown == "1",
        "options": options
    })
}

fn row_json(status: StatusCode, body: Value) -> Response {
    let mut response = Response::new(axum::body::Body::from(body.to_string()));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

async fn row_save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    Path((model_key, id)): Path<(String, Uuid)>,
    body: Bytes,
) -> Response {
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let user = match require_post(&state, &headers, &body).await {
        Ok(user) => user,
        Err(()) => {
            return row_json(
                StatusCode::FORBIDDEN,
                json!({ "ok": false, "error": catalog(&locale).get("csrf").and_then(Value::as_str).unwrap_or("Forbidden") }),
            )
        }
    };
    let Some(model) = registry.find(&model_key) else {
        return row_json(StatusCode::NOT_FOUND, json!({ "ok": false }));
    };
    let allowed = registry.list_editable_of(model.meta());
    if allowed.is_empty()
        || gate(&state, &registry, &user, model.as_ref(), "update")
            .await
            .is_err()
    {
        return row_json(StatusCode::NOT_FOUND, json!({ "ok": false }));
    }
    let Some(current) = read_visible(&state, &user, model.as_ref(), id, "update").await else {
        return row_json(StatusCode::NOT_FOUND, json!({ "ok": false }));
    };
    let Some(current_object) = current.as_object() else {
        return row_json(StatusCode::UNPROCESSABLE_ENTITY, json!({ "ok": false }));
    };
    let mut object = Map::new();
    for field in model.meta().fields {
        if field.name == "id" {
            continue;
        }
        if let Some(value) = current_object.get(field.name) {
            object.insert(field.name.to_owned(), value.clone());
        }
    }
    let form = match row_form(&body) {
        Ok(form) => form,
        Err(()) => {
            return row_json(
                StatusCode::UNPROCESSABLE_ENTITY,
                json!({ "ok": false, "error": catalog(&locale).get("forbidden").and_then(Value::as_str).unwrap_or("Rejected") }),
            )
        }
    };
    if apply_row_fields(&mut object, model.columns(), allowed, &form).is_err() {
        return row_json(
            StatusCode::UNPROCESSABLE_ENTITY,
            json!({ "ok": false, "error": catalog(&locale).get("forbidden").and_then(Value::as_str).unwrap_or("Rejected") }),
        );
    }
    match model
        .update(
            &state.db,
            id,
            Value::Object(object),
            &access_scope(model.as_ref(), &user, true),
            &user,
        )
        .await
    {
        Ok(_) => row_json(StatusCode::OK, json!({ "ok": true })),
        Err(FexuError::Validation(message)) => row_json(
            StatusCode::UNPROCESSABLE_ENTITY,
            json!({ "ok": false, "error": validation_text(&locale, &message) }),
        ),
        Err(_) => row_json(
            StatusCode::UNPROCESSABLE_ENTITY,
            json!({ "ok": false, "error": catalog(&locale).get("forbidden").and_then(Value::as_str).unwrap_or("Rejected") }),
        ),
    }
}

async fn model_update(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    Path((model_key, id)): Path<(String, Uuid)>,
    body: Bytes,
) -> Response {
    save_form(&state, &headers, &registry, &model_key, Some(id), &body).await
}

async fn model_delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    Path((model_key, id)): Path<(String, Uuid)>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let user = match require_get(
        &state,
        &headers,
        &registry,
        &format!("/admin/models/{model_key}/{id}/delete"),
    )
    .await
    {
        Ok(user) => user,
        Err(response) => return response,
    };
    let Some(model) = registry.find(&model_key) else {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    };
    if gate(&state, &registry, &user, model.as_ref(), "delete")
        .await
        .is_err()
    {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    }
    let Some(record) = read_visible(&state, &user, model.as_ref(), id, "delete").await else {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    };
    let named = display_column(registry.field_slice(model.meta()), model.columns())
        .and_then(|name| record.get(name))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or("")
        .to_owned();
    let csrf = session_csrf(state.auth.as_ref().unwrap(), &headers).await;
    page_as(
        &state,
        &user,
        &registry,
        "delete.html",
        &locale,
        theme_of(&headers),
        catalog(&locale)["delete"].as_str().unwrap_or("Delete"),
        true,
        user.is_superuser,
        &user.email,
        &csrf,
        &format!("/admin/models/{model_key}"),
        "",
        "",
        json!({
            "action": format!("/admin/models/{model_key}/{id}/delete"),
            "cancel": format!("/admin/models/{model_key}/{id}"),
            "label": if named.is_empty() { id.to_string() } else { named },
            "record_id": id.to_string(),
            "model_label": model.meta().label,
            "list_path": format!("/admin/models/{model_key}"),
            "popup": query.get("popup").is_some_and(|value| value == "1"),
            "nav": !query.get("popup").is_some_and(|value| value == "1")
        }),
    )
    .await
}

async fn model_remove(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    Path((model_key, id)): Path<(String, Uuid)>,
    body: Bytes,
) -> Response {
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let user = match require_post(&state, &headers, &body).await {
        Ok(user) => user,
        Err(_) => {
            return error_page(
                &state,
                &headers,
                &registry,
                &locale,
                StatusCode::FORBIDDEN,
                "csrf",
                true,
                "",
            )
            .await
        }
    };
    let Some(model) = registry.find(&model_key) else {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    };
    if gate(&state, &registry, &user, model.as_ref(), "delete")
        .await
        .is_err()
        || read_visible(&state, &user, model.as_ref(), id, "delete")
            .await
            .is_none()
    {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    }
    let scope = access_scope(model.as_ref(), &user, true);
    match model.remove(&state.db, id, &scope, &user).await {
        Ok(fexu_db::DeleteOutcome::Deleted) => {
            let _ = crate::files::delete_record(&state.db, &model_key, id).await;
            redirect(&format!("/admin/models/{model_key}?notice=deleted"))
        }
        _ => {
            error_page(
                &state,
                &headers,
                &registry,
                &locale,
                StatusCode::NOT_FOUND,
                "not_found",
                true,
                &user.email,
            )
            .await
        }
    }
}

async fn users(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let user = match require_roles(&state, &headers, &registry, "/admin/directory/users").await {
        Ok(user) => user,
        Err(response) => return response,
    };
    let auth = state.auth.as_ref().unwrap();
    let listed = auth.list_users().await.unwrap_or_default();
    let mut emails: Vec<String> = listed
        .iter()
        .map(|candidate| candidate.email.clone())
        .collect();
    emails.sort();
    let mut role_choices: Vec<String> = auth
        .list_roles()
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|(_, name)| name)
        .collect();
    role_choices.sort();
    let links = auth.list_user_role_names().await.unwrap_or_default();
    let mut roles: HashMap<Uuid, Vec<String>> = HashMap::new();
    for (id, name) in links {
        roles.entry(id).or_default().push(name);
    }
    let display_names = auth
        .display_names()
        .await
        .unwrap_or_default()
        .into_iter()
        .collect::<HashMap<Uuid, String>>();
    let query = params.get("q").cloned().unwrap_or_default();
    let needle = query.to_ascii_lowercase();
    let mut rows: Vec<_> = listed
        .into_iter()
        .filter(|candidate| {
            query.is_empty()
                || candidate.email.to_ascii_lowercase().contains(&needle)
                || display_names
                    .get(&candidate.id)
                    .is_some_and(|name| name.to_ascii_lowercase().contains(&needle))
        })
        .collect();
    let descending = params.get("dir").map(String::as_str) == Some("desc");
    rows.sort_by(|left, right| {
        if descending {
            right.email.cmp(&left.email)
        } else {
            left.email.cmp(&right.email)
        }
    });
    let page_number = params
        .get("page")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(1)
        .max(1);
    let total = rows.len();
    let start = (page_number - 1) * 25;
    let slice = rows.into_iter().skip(start).take(25);
    let users: Vec<Value> = slice
        .map(|candidate| {
            let mut names = roles.remove(&candidate.id).unwrap_or_default();
            names.sort();
            json!({
                "id": candidate.id,
                "email": candidate.email,
                "name": display_names.get(&candidate.id).cloned().unwrap_or_default(),
                "is_active": candidate.is_active,
                "is_staff": candidate.is_staff,
                "is_superuser": candidate.is_superuser,
                "roles": names.join(", "),
                "can_edit": user.is_superuser,
                "can_assign": true,
                "mutable": user.is_superuser && !candidate.is_superuser && candidate.id != user.id
            })
        })
        .collect();
    let shown = users.len();
    let csrf = session_csrf(auth, &headers).await;
    page_as(
        &state,
        &user,
        &registry,
        "users.html",
        &locale,
        theme_of(&headers),
        catalog(&locale)["users"].as_str().unwrap_or("Users"),
        true,
        true,
        &user.email,
        &csrf,
        "/admin/directory/users",
        &flash(
            &locale,
            params.get("notice").map(String::as_str).unwrap_or(""),
        ),
        "",
        json!({
            "users": users,
            "can_add": user.is_superuser,
            "query": query,
            "total": total,
            "shown": shown,
            "page": page_number,
            "emails": emails,
            "role_choices": role_choices,
            "email_mark": if descending { "↓" } else { "↑" },
            "email_state": if descending { "descending" } else { "ascending" },
            "email_sort": format!(
                "/admin/directory/users?q={}&sort=email&dir={}",
                url_encode(&query),
                if descending { "asc" } else { "desc" }
            ),
            "email_dir": if descending { "asc" } else { "desc" },
            "previous": (page_number > 1).then(|| format!("/admin/directory/users?q={}&dir={}&page={}", url_encode(&query), if descending { "desc" } else { "asc" }, page_number - 1)),
            "next_page": (start + 25 < total).then(|| format!("/admin/directory/users?q={}&dir={}&page={}", url_encode(&query), if descending { "desc" } else { "asc" }, page_number + 1))
        }),
    )
    .await
}

async fn user_active(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    Path(id): Path<Uuid>,
    body: Bytes,
) -> Response {
    mutate_user(&state, &headers, &registry, id, &body, true).await
}

async fn user_staff(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    Path(id): Path<Uuid>,
    body: Bytes,
) -> Response {
    mutate_user(&state, &headers, &registry, id, &body, false).await
}

async fn roles(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
) -> Response {
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let user = match require_roles(&state, &headers, &registry, "/admin/directory/roles").await {
        Ok(user) => user,
        Err(response) => return response,
    };
    let listed = state
        .auth
        .as_ref()
        .unwrap()
        .list_roles()
        .await
        .unwrap_or_default();
    let roles: Vec<Value> = listed
        .into_iter()
        .map(|(id, name)| {
            json!({
                "name": name,
                "href": format!("/admin/directory/roles/{id}"),
                "history": history_href("_fexu_users", &name)
            })
        })
        .collect();
    let csrf = session_csrf(state.auth.as_ref().unwrap(), &headers).await;
    page_as(
        &state,
        &user,
        &registry,
        "roles.html",
        &locale,
        theme_of(&headers),
        catalog(&locale)["roles"].as_str().unwrap_or("Roles"),
        true,
        true,
        &user.email,
        &csrf,
        "/admin/directory/roles",
        "",
        "",
        json!({ "roles": roles }),
    )
    .await
}

async fn role_new(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
) -> Response {
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let user = match require_roles(&state, &headers, &registry, "/admin/directory/roles/new").await
    {
        Ok(user) => user,
        Err(response) => return response,
    };
    let csrf = session_csrf(state.auth.as_ref().unwrap(), &headers).await;
    page_as(
        &state,
        &user,
        &registry,
        "role_new.html",
        &locale,
        theme_of(&headers),
        catalog(&locale)["add_role"]
            .as_str()
            .unwrap_or("New record"),
        true,
        true,
        &user.email,
        &csrf,
        "/admin/directory/roles/new",
        "",
        "",
        json!({}),
    )
    .await
}

async fn role_create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    body: Bytes,
) -> Response {
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let user = match require_roles_post(&state, &headers, &registry, &body).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    let name = parse_form(&body).get("name").cloned().unwrap_or_default();
    if !role_label_ok(&name) {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::UNPROCESSABLE_ENTITY,
            "forbidden",
            true,
            &user.email,
        )
        .await;
    }
    if state
        .auth
        .as_ref()
        .unwrap()
        .ensure_role(&name)
        .await
        .is_err()
    {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::UNPROCESSABLE_ENTITY,
            "forbidden",
            true,
            &user.email,
        )
        .await;
    }
    redirect("/admin/directory/roles?notice=updated")
}

async fn permissions_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    role_permissions_page(state, headers, registry, params, None).await
}

async fn role_detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    Path(id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    role_permissions_page(state, headers, registry, params, Some(id)).await
}

async fn role_permissions_page(
    state: AppState,
    headers: HeaderMap,
    registry: AdminRegistry,
    params: HashMap<String, String>,
    record_id: Option<Uuid>,
) -> Response {
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let user =
        match require_roles(&state, &headers, &registry, "/admin/directory/permissions").await {
            Ok(user) => user,
            Err(response) => return response,
        };
    let auth = state.auth.as_ref().unwrap();
    let listed = auth.list_roles().await.unwrap_or_default();
    let mut roles = Vec::new();
    for (id, name) in listed {
        if record_id.is_some_and(|selected| selected != id) {
            continue;
        }
        if record_id.is_none() {
            if let Some(selected) = params.get("role") {
                if selected != &name {
                    continue;
                }
            }
        }
        let permissions = auth.list_role_permissions(&name).await.unwrap_or_default();
        let catalog = permission_catalog(&registry, &locale);
        let mut known = std::collections::BTreeSet::new();
        let rows: Vec<Value> = catalog
            .iter()
            .map(|row| {
                let key = row
                    .get("key")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned();
                let mut row = row.clone();
                for action in ["create", "read", "update", "delete"] {
                    let code = format!("{key}.{action}");
                    known.insert(code.clone());
                    let granted = permissions.iter().any(|item| item == &code);
                    if let Some(object) = row.as_object_mut() {
                        object.insert(action.to_owned(), json!(granted));
                    }
                }
                row
            })
            .collect();
        let extra: Vec<&String> = permissions
            .iter()
            .filter(|code| !known.contains(*code))
            .collect();
        roles.push(json!({ "id": id, "name": name, "permissions": permissions, "rows": rows, "extra": extra }));
    }
    if record_id.is_some() && roles.is_empty() {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    }
    let csrf = session_csrf(auth, &headers).await;
    let history = if record_id.is_some() {
        record_history(
            &state,
            &user,
            &locale,
            "_fexu_users",
            roles
                .first()
                .and_then(|role| role["name"].as_str())
                .unwrap_or(""),
        )
        .await
    } else {
        RecordHistory::none()
    };
    page_as(
        &state,
        &user,
        &registry,
        if record_id.is_some() { "role_detail.html" } else { "permissions.html" },
        &locale,
        theme_of(&headers),
        catalog(&locale)["permissions"]
            .as_str()
            .unwrap_or("Permissions"),
        true,
        true,
        &user.email,
        &csrf,
        "/admin/directory/permissions",
        &flash(&locale, params.get("notice").map(String::as_str).unwrap_or("")),
        "",
        json!({ "roles": roles, "record_role": record_id.is_some(),
            "history": history.events,
            "history_more": history.more,
            "history_href": roles.first().and_then(|role| role["name"].as_str()).map(|name| history_href("_fexu_users", name)).unwrap_or_default() }),
    )
    .await
}

async fn permission_change(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    Path(role): Path<String>,
    body: Bytes,
) -> Response {
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let user = match require_roles_post(&state, &headers, &registry, &body).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    let form = parse_form(&body);
    let auth = state.auth.as_ref().unwrap();
    let listed = auth.list_roles().await.unwrap_or_default();
    let role = if let Ok(id) = uuid::Uuid::parse_str(&role) {
        listed
            .iter()
            .find(|(role_id, _)| role_id == &id)
            .map(|(_, name)| name.clone())
            .unwrap_or_default()
    } else {
        role
    };
    if form.get("matrix").map(String::as_str) == Some("1") {
        return save_permission_matrix(&state, &headers, &registry, &locale, &user, &role, &form)
            .await;
    }
    let code = form.get("code").cloned().unwrap_or_default();
    if !role_label_ok(&role)
        || !identifier(&code)
        || !matches!(
            form.get("action").map(String::as_str),
            Some("grant" | "revoke")
        )
        || form.keys().any(|key| {
            !matches!(
                key.as_str(),
                "csrf_token" | "confirm" | "code" | "action" | "return_to"
            )
        })
    {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::UNPROCESSABLE_ENTITY,
            "forbidden",
            true,
            &user.email,
        )
        .await;
    }
    if let Err(code) = confirmed(&body) {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::UNPROCESSABLE_ENTITY,
            code,
            true,
            &user.email,
        )
        .await;
    }
    let auth = state.auth.as_ref().unwrap();
    let grant = form.get("action").map(String::as_str) == Some("grant");
    let result = crate::directory::grant_permission(auth, user.id, &role, &code, grant).await;
    if result.is_err() {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::UNPROCESSABLE_ENTITY,
            "forbidden",
            true,
            &user.email,
        )
        .await;
    }
    if form.get("return_to").map(String::as_str) == Some("record") {
        if let Ok(roles) = auth.list_roles().await {
            if let Some((id, _)) = roles.iter().find(|(_, name)| name == &role) {
                return redirect(&format!("/admin/directory/roles/{id}?notice=updated"));
            }
        }
    }
    redirect("/admin/directory/permissions?notice=updated")
}

async fn assignment(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    body: Bytes,
) -> Response {
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let user = match require_roles_post(&state, &headers, &registry, &body).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    let form = parse_form(&body);
    let email = form.get("email").cloned().unwrap_or_default();
    let role = form.get("role").cloned().unwrap_or_default();
    if !role_label_ok(&role) {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::UNPROCESSABLE_ENTITY,
            "forbidden",
            true,
            &user.email,
        )
        .await;
    }
    let auth = state.auth.as_ref().unwrap();
    let known = auth.list_roles().await.unwrap_or_default();
    if !known.iter().any(|(_, name)| name == &role) {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::UNPROCESSABLE_ENTITY,
            "unknown_role",
            true,
            &user.email,
        )
        .await;
    }
    if let Err(code) = confirmed(&body) {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::UNPROCESSABLE_ENTITY,
            code,
            true,
            &user.email,
        )
        .await;
    }
    let result = crate::directory::assign_role(
        auth,
        user.id,
        &email,
        &role,
        form.get("action").map(String::as_str) != Some("revoke"),
    )
    .await;
    if result.is_err() {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::UNPROCESSABLE_ENTITY,
            "forbidden",
            true,
            &user.email,
        )
        .await;
    }
    redirect("/admin/directory/users?notice=updated")
}

async fn model_pick(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    Path((model_key, field)): Path<(String, String)>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    pick_page(
        &state, &headers, &registry, &model_key, None, &field, &params,
    )
    .await
}

async fn model_record_pick(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    Path((model_key, id, field)): Path<(String, Uuid, String)>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    pick_page(
        &state,
        &headers,
        &registry,
        &model_key,
        Some(id),
        &field,
        &params,
    )
    .await
}

async fn pick_page(
    state: &AppState,
    headers: &HeaderMap,
    registry: &AdminRegistry,
    model_key: &str,
    record_id: Option<Uuid>,
    field: &str,
    params: &HashMap<String, String>,
) -> Response {
    let locale = locale_of(
        headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let back = match record_id {
        Some(id) => format!("/admin/models/{model_key}/{id}"),
        None => format!("/admin/models/{model_key}/new"),
    };
    let user = match require_get(state, headers, registry, &back).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    let Some(model) = registry.find(model_key) else {
        return error_page(
            state,
            headers,
            registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    };
    let action_name = if record_id.is_some() {
        "update"
    } else {
        "create"
    };
    if gate(state, registry, &user, model.as_ref(), action_name)
        .await
        .is_err()
    {
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
    if let Some(id) = record_id {
        if load_for_form(state, headers, registry, model_key, id)
            .await
            .is_none()
        {
            return error_page(
                state,
                headers,
                registry,
                &locale,
                StatusCode::NOT_FOUND,
                "not_found",
                true,
                &user.email,
            )
            .await;
        }
    }
    let Some(column) = model
        .columns()
        .iter()
        .find(|column| column.name == field && column.foreign_table.is_some())
    else {
        return error_page(
            state,
            headers,
            registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    };
    if model
        .meta()
        .fields
        .iter()
        .any(|item| item.name == field && (item.hidden || item.readonly || sensitive(item.name)))
    {
        return error_page(
            state,
            headers,
            registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    }
    let Some(target) = column
        .foreign_table
        .and_then(|table| registry.model_for_table(table))
    else {
        return error_page(
            state,
            headers,
            registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    };
    if list_blocked(target.as_ref())
        || permit(state.auth.as_ref().unwrap(), &user, target.meta(), "read")
            .await
            .is_err()
    {
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
    let Some(label) = display_column(registry.field_slice(target.meta()), target.columns()) else {
        return error_page(
            state,
            headers,
            registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    };
    let page_number = params
        .get("page")
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(1)
        .clamp(1, 10_000);
    let page_size = u64::from(registry.per_page_of(target.meta()).clamp(1, 100));
    let search_columns: Vec<&str> = registry
        .field_slice(target.meta())
        .iter()
        .filter(|item| item.searchable && !item.hidden && !sensitive(item.name))
        .map(|item| item.name)
        .collect();
    let query = params.get("q").cloned().unwrap_or_default();
    if query.chars().count() > 200 || (!query.is_empty() && search_columns.is_empty()) {
        return error_page(
            state,
            headers,
            registry,
            &locale,
            StatusCode::UNPROCESSABLE_ENTITY,
            "forbidden",
            true,
            &user.email,
        )
        .await;
    }
    let scope = access_scope(target.as_ref(), &user, false);
    let listed = target
        .page(
            &state.db,
            &search_columns,
            (!query.is_empty()).then_some(query.as_str()),
            label,
            false,
            &[],
            &[],
            u64::from(page_number - 1) * page_size,
            page_size,
            &scope,
        )
        .await;
    let (rows, has_more, total) = match listed {
        Ok(value) => value,
        Err(_) => {
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
            .await
        }
    };
    let picker = match record_id {
        Some(id) => format!("/admin/models/{model_key}/{id}/pick/{field}"),
        None => format!("/admin/models/{model_key}/pick/{field}"),
    };
    let mut preserved = if query.is_empty() {
        String::new()
    } else {
        format!("q={}", url_encode(&query))
    };
    if let Some(draft) = params
        .get("draft")
        .filter(|value| Uuid::parse_str(value).is_ok())
    {
        if !preserved.is_empty() {
            preserved.push('&');
        }
        preserved.push_str("draft=");
        preserved.push_str(&url_encode(draft));
    }
    let choices: Vec<Value> = rows
        .iter()
        .filter_map(|row| {
            let id = row.get("id")?.as_str()?;
            let text = row.get(label)?.as_str().unwrap_or("");
            Some(json!({
                "label": text,
                "href": choice_href(&back, field, id, params.get("draft").map(String::as_str))
            }))
        })
        .collect();
    let join = if preserved.is_empty() { "" } else { "&" };
    let csrf = session_csrf(state.auth.as_ref().unwrap(), headers).await;
    page_as(
        state,
        &user,
        registry,
        "pick.html",
        &locale,
        theme_of(headers),
        catalog(&locale)
            .get("pick_heading")
            .and_then(Value::as_str)
            .unwrap_or("Choose"),
        true,
        user.is_superuser,
        &user.email,
        &csrf,
        &picker,
        "",
        "",
        json!({
            "back": back,
            "action": picker,
            "query": query,
            "rows": choices,
            "total": total,
            "page": page_number,
            "previous": (page_number > 1).then(|| format!("{picker}?{preserved}{join}page={}", page_number - 1)),
            "next_page": has_more.then(|| format!("{picker}?{preserved}{join}page={}", page_number + 1))
        }),
    )
    .await
}

fn validation_field(fields: &[fexu_core::AdminFieldMeta], message: &str) -> String {
    let tokens = message
        .split(|char: char| !char.is_ascii_alphanumeric() && char != '_')
        .filter(|token| !token.is_empty());
    let mut matched = String::new();
    for token in tokens {
        if fields.iter().any(|field| field.name == token) && matched.is_empty() {
            matched = token.to_owned();
        }
    }
    matched
}

fn posted_link_values(repeated: &HashMap<String, Vec<String>>) -> Map<String, Value> {
    let mut posted = Map::new();
    let Some(names) = repeated.get("m2m_present") else {
        return posted;
    };
    for name in names {
        if posted.contains_key(name) {
            continue;
        }
        let values = repeated
            .get(&format!("m2m.{name}"))
            .cloned()
            .unwrap_or_default();
        posted.insert(name.clone(), json!(values));
    }
    posted
}

struct PlannedLink {
    name: String,
    link: fexu_db::ManyToMany,
    next: Vec<Uuid>,
    changed: bool,
}

async fn plan_memberships(
    state: &AppState,
    registry: &AdminRegistry,
    user: &AuthUser,
    model: &dyn ManagedModel,
    parent: Option<Uuid>,
    repeated: &HashMap<String, Vec<String>>,
) -> Result<Vec<PlannedLink>, (String, &'static str)> {
    let links = registry.links_of(model.meta());
    if links.is_empty() {
        return Ok(Vec::new());
    }
    let present = repeated.get("m2m_present").cloned().unwrap_or_default();
    let mut planned = Vec::new();
    for link in links {
        if !present.iter().any(|name| name == link.name) {
            continue;
        }
        let requested = posted_members(
            repeated
                .get(&format!("m2m.{}", link.name))
                .map(Vec::as_slice)
                .unwrap_or(&[]),
        )
        .map_err(|code| (link.name.to_owned(), code))?;
        let current = match parent {
            Some(parent) => current_members(&state.db, link.relation(), parent)
                .await
                .map_err(|_| (link.name.to_owned(), "admin.relation"))?,
            None => Vec::new(),
        };
        let Some(child) = registry.find(link.child_key) else {
            return Err((link.name.to_owned(), "admin.relation"));
        };
        let mut candidates = current.clone();
        for id in &requested {
            if !candidates.contains(id) {
                candidates.push(*id);
            }
        }
        let visible = visible_member_ids(state, user, child.as_ref(), &candidates).await;
        let next = scoped_membership(&current, &visible, &requested)
            .map_err(|code| (link.name.to_owned(), code))?;
        let changed = !same_ids(&current, &next);
        planned.push(PlannedLink {
            name: link.name.to_owned(),
            link: link.relation(),
            next,
            changed,
        });
    }
    Ok(planned)
}

fn same_ids(left: &[Uuid], right: &[Uuid]) -> bool {
    let mut left = left.to_vec();
    let mut right = right.to_vec();
    left.sort();
    right.sort();
    left == right
}

async fn visible_member_ids(
    state: &AppState,
    user: &AuthUser,
    child: &dyn ManagedModel,
    ids: &[Uuid],
) -> Vec<Uuid> {
    let Some(auth) = state.auth.as_ref() else {
        return Vec::new();
    };
    if list_blocked(child) || permit(auth, user, child.meta(), "read").await.is_err() {
        return Vec::new();
    }
    let mut visible = Vec::new();
    for id in ids {
        if read_visible(state, user, child, *id, "read")
            .await
            .is_some()
        {
            visible.push(*id);
        }
    }
    visible
}

#[allow(clippy::too_many_arguments)]
async fn membership_widgets(
    state: &AppState,
    registry: &AdminRegistry,
    user: &AuthUser,
    model: &dyn ManagedModel,
    parent: Option<Uuid>,
    posted: Option<&Value>,
    errors: Option<&Value>,
    locale: &str,
) -> Vec<Value> {
    let hint = catalog(locale)
        .get("m2m_hint")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let mut widgets = Vec::new();
    for link in registry.links_of(model.meta()) {
        let error = errors
            .and_then(|value| value.get(link.name))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let Some(child) = registry.find(link.child_key) else {
            widgets.push(empty_membership(link, &hint, &error, false, true));
            continue;
        };
        let readable = match state.auth.as_ref() {
            Some(auth) => {
                !list_blocked(child.as_ref())
                    && permit(auth, user, child.meta(), "read").await.is_ok()
            }
            None => false,
        };
        if !readable {
            widgets.push(empty_membership(link, &hint, &error, true, true));
            continue;
        }
        let chosen =
            match chosen_member_ids(state, user, child.as_ref(), link, parent, posted).await {
                Ok(chosen) => chosen,
                Err(()) => {
                    let failure = if error.is_empty() {
                        catalog(locale)
                            .get("m2m_invalid")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned()
                    } else {
                        error
                    };
                    widgets.push(empty_membership(link, &hint, &failure, false, false));
                    continue;
                }
            };
        let mut options = Vec::new();
        let mut truncated = false;
        if let Some(label) = display_column(registry.field_slice(child.meta()), child.columns()) {
            let scope = access_scope(child.as_ref(), user, false);
            if let Ok(page) = lookup_page(
                &state.db,
                child.descriptor().table,
                child.columns(),
                label,
                None,
                1,
                100,
                &scope,
            )
            .await
            {
                truncated = page.total > i64::try_from(page.rows.len()).unwrap_or(i64::MAX);
                for row in &page.rows {
                    if let Some(option) = member_option(row, label, &chosen) {
                        options.push(option);
                    }
                }
            }
            for id in &chosen {
                let shown = options.iter().any(|option| {
                    option
                        .get("id")
                        .and_then(Value::as_str)
                        .and_then(|text| Uuid::parse_str(text).ok())
                        == Some(*id)
                });
                if shown {
                    continue;
                }
                let Some(record) = read_visible(state, user, child.as_ref(), *id, "read").await
                else {
                    continue;
                };
                if let Some(option) = member_option(&record, label, &chosen) {
                    options.push(option);
                }
            }
        }
        widgets.push(json!({
            "name": link.name,
            "label": link.label,
            "hint": hint,
            "ready": true,
            "blocked": false,
            "truncated": truncated,
            "invalid": !error.is_empty(),
            "error": error,
            "options": options,
        }));
    }
    widgets
}

fn empty_membership(link: &ManyLink, hint: &str, error: &str, ready: bool, blocked: bool) -> Value {
    json!({
        "name": link.name,
        "label": link.label,
        "hint": hint,
        "ready": ready,
        "blocked": blocked,
        "truncated": false,
        "invalid": !error.is_empty(),
        "error": error,
        "options": [],
    })
}

async fn chosen_member_ids(
    state: &AppState,
    user: &AuthUser,
    child: &dyn ManagedModel,
    link: &ManyLink,
    parent: Option<Uuid>,
    posted: Option<&Value>,
) -> Result<Vec<Uuid>, ()> {
    let ids = if let Some(values) = posted
        .and_then(|value| value.get(link.name))
        .and_then(Value::as_array)
    {
        let mut parsed = Vec::new();
        for value in values {
            let Some(text) = value.as_str() else {
                continue;
            };
            let Ok(id) = Uuid::parse_str(text.trim()) else {
                continue;
            };
            if !parsed.contains(&id) {
                parsed.push(id);
            }
        }
        parsed
    } else if let Some(parent) = parent {
        current_members(&state.db, link.relation(), parent)
            .await
            .map_err(|_| ())?
    } else {
        Vec::new()
    };
    Ok(visible_member_ids(state, user, child, &ids).await)
}

fn member_option(row: &Value, label: &str, selected: &[Uuid]) -> Option<Value> {
    let id = Uuid::parse_str(row.get("id")?.as_str()?).ok()?;
    let text = row.get(label)?.as_str()?;
    Some(json!({
        "id": id.to_string(),
        "label": text,
        "selected": selected.contains(&id),
    }))
}

struct FormFeedback<'a> {
    notice: &'a str,
    alert: &'a str,
}

async fn form_page(
    state: &AppState,
    headers: &HeaderMap,
    registry: &AdminRegistry,
    model_key: &str,
    id: Option<Uuid>,
    values: Map<String, Value>,
    feedback: FormFeedback<'_>,
) -> Response {
    let locale = locale_of(
        headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let back = format!("/admin/models/{model_key}");
    let user = match require_get(state, headers, registry, &back).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    let Some(model) = registry.find(model_key) else {
        return error_page(
            state,
            headers,
            registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    };
    let action_name = if id.is_some() { "update" } else { "create" };
    if gate(state, registry, &user, model.as_ref(), action_name)
        .await
        .is_err()
    {
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
    if id.is_some() && values.is_empty() {
        return error_page(
            state,
            headers,
            registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    }
    let mut values = values;
    let popup = values
        .remove("_popup")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let inline_query: HashMap<String, String> = values
        .remove("_inline_query")
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();
    let posted_membership = values.remove("_m2m");
    let membership_errors = values.remove("_m2m_error");
    let draft_id = take_draft(state, &user, model_key, id, &mut values).await;
    if let Some(id) = id {
        values
            .entry("id".to_owned())
            .or_insert_with(|| json!(id.to_string()));
    }
    let custom_action = values
        .get("_action")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let csrf = session_csrf(state.auth.as_ref().unwrap(), headers).await;
    let fields = input_fields(&locale, registry, state, &user, model.as_ref(), &values).await;
    let sections = form_sections(
        registry.fieldsets_of(model.meta()),
        &fields,
        registry.form_layout(model.meta().key),
    );
    let memberships = membership_widgets(
        state,
        registry,
        &user,
        model.as_ref(),
        id,
        posted_membership.as_ref(),
        membership_errors.as_ref(),
        &locale,
    )
    .await;
    let mut error_fields = Vec::new();
    for field in &fields {
        let error = field.get("error").and_then(Value::as_str).unwrap_or("");
        if error.is_empty() {
            continue;
        }
        error_fields.push(json!({
            "name": field.get("name").and_then(Value::as_str).unwrap_or(""),
            "label": field.get("label").and_then(Value::as_str).unwrap_or(""),
            "error": error,
        }));
    }
    for group in &memberships {
        let error = group.get("error").and_then(Value::as_str).unwrap_or("");
        if error.is_empty() {
            continue;
        }
        error_fields.push(json!({
            "name": format!("m2m-{}", group.get("name").and_then(Value::as_str).unwrap_or("")),
            "label": group.get("label").and_then(Value::as_str).unwrap_or(""),
            "error": error,
        }));
    }
    let (inline_groups, basis) = if model.meta().inlines.iter().any(|item| item.form) {
        crate::compose::groups(
            &state.db,
            state.auth.as_ref().unwrap(),
            &user,
            model.as_ref(),
            |key| registry.find(key).cloned(),
            id,
            &values,
        )
        .await
    } else {
        (Vec::new(), String::new())
    };
    let action = custom_action.unwrap_or_else(|| match id {
        Some(id) => format!("/admin/models/{model_key}/{id}"),
        None => format!("/admin/models/{model_key}/new"),
    });
    let list_path = back.clone();
    let cancel = values
        .get("_cancel")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| back.clone());
    let history = match id {
        Some(record_id) => {
            record_history(
                state,
                &user,
                &locale,
                model.meta().key,
                &record_id.to_string(),
            )
            .await
        }
        None => RecordHistory::none(),
    };
    let mut response = page_as(
        state,
        &user,
        registry,
        "form.html",
        &locale,
        theme_of(headers),
        model.meta().label,
        true,
        user.is_superuser,
        &user.email,
        &csrf,
        &back,
        feedback.notice,
        feedback.alert,
        json!({
            "model_key": model_key,
            "existing": id.is_some(),
            "can_create": gate(state, registry, &user, model.as_ref(), "create").await.is_ok(),
            "inlines": if let Some(record_id) = id {
                inline_sections(state, registry, &user, model.as_ref(), record_id, &inline_query, true, &locale).await
            } else { Vec::new() },
            "delete_href": if let Some(record_id) = id {
                (gate(state, registry, &user, model.as_ref(), "delete").await.is_ok()
                    && read_visible(state, &user, model.as_ref(), record_id, "delete").await.is_some())
                    .then(|| format!("/admin/models/{model_key}/{record_id}/delete"))
            } else {
                None
            },
            "history_href": id.map(|value| history_href(model.meta().key, &value.to_string())).unwrap_or_default(),
            "history": history.events,
            "history_more": history.more,
            "model_label": model.meta().label,
            "list_path": list_path,
            "action": action,
            "cancel": cancel,
            "draft": draft_id,
            "sections": sections,
            "inline_groups": inline_groups,
            "memberships": memberships,
            "error_fields": error_fields,
            "basis": basis,
            "popup": popup,
            "nav": !popup,
            "uploads": registry.widgets().iter().any(|widget| {
                widget.model_key == model_key
                    && matches!(widget.kind, crate::widgets::WidgetKind::Image)
            })
        }),
    )
    .await;
    if !feedback.alert.is_empty() {
        *response.status_mut() = StatusCode::UNPROCESSABLE_ENTITY;
    }
    response
}

struct ModelUpload {
    filename: String,
    bytes: Vec<u8>,
}

struct PostedModel {
    fields: HashMap<String, String>,
    repeated: HashMap<String, Vec<String>>,
    uploads: Vec<(String, ModelUpload)>,
}

fn read_model_body(headers: &HeaderMap, body: &Bytes) -> Result<PostedModel, ()> {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    if !content_type
        .to_ascii_lowercase()
        .starts_with("multipart/form-data")
    {
        let mut fields = HashMap::new();
        let mut repeated = HashMap::new();
        for pair in String::from_utf8_lossy(body).split('&') {
            if pair.is_empty() {
                continue;
            }
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            store_text(&mut fields, &mut repeated, decode(key), decode(value))?;
        }
        return Ok(PostedModel {
            fields,
            repeated,
            uploads: Vec::new(),
        });
    }
    let parts = fexu_storage::read_multipart_repeated(content_type, body).map_err(|_| ())?;
    let mut fields = HashMap::new();
    let mut repeated = HashMap::new();
    let mut uploads = Vec::new();
    let mut file_names = std::collections::HashSet::new();
    for part in parts {
        if let Some(field) = part.name.strip_prefix("file.") {
            if field.is_empty() || !file_names.insert(field.to_owned()) {
                return Err(());
            }
            if let Some(filename) = part.filename.filter(|name| !name.is_empty()) {
                if !part.bytes.is_empty() {
                    uploads.push((
                        field.to_owned(),
                        ModelUpload {
                            filename,
                            bytes: part.bytes,
                        },
                    ));
                }
            }
            continue;
        }
        if part.filename.is_some() {
            return Err(());
        }
        let text = String::from_utf8(part.bytes).map_err(|_| ())?;
        store_text(&mut fields, &mut repeated, part.name, text)?;
    }
    Ok(PostedModel {
        fields,
        repeated,
        uploads,
    })
}

fn store_text(
    fields: &mut HashMap<String, String>,
    repeated: &mut HashMap<String, Vec<String>>,
    name: String,
    text: String,
) -> Result<(), ()> {
    if name == "m2m_present" || name.starts_with("m2m.") {
        if !text.is_empty() {
            repeated.entry(name).or_default().push(text);
        }
        return Ok(());
    }
    match fields.get(&name).map(String::as_str) {
        None => {
            fields.insert(name, text);
            Ok(())
        }
        Some("false") if text == "true" => {
            fields.insert(name, text);
            Ok(())
        }
        _ => Err(()),
    }
}

fn apply_image_uploads(
    registry: &AdminRegistry,
    model_key: &str,
    object: &mut Map<String, Value>,
    raw: &mut Map<String, Value>,
    uploads: &[(String, ModelUpload)],
) -> Result<(), String> {
    for (field, upload) in uploads {
        let allowed = registry.widgets().iter().any(|widget| {
            widget.model_key == model_key
                && widget.field == field
                && matches!(widget.kind, crate::widgets::WidgetKind::Image)
        });
        if !allowed || crate::files::image_type(&upload.bytes).is_none() {
            return Err(format!("field:{field}: value is not an image"));
        }
        let name = fexu_storage::sanitize_name(&upload.filename);
        if name.is_empty() || name.chars().count() > 200 {
            return Err(format!("field:{field}: value is not an image"));
        }
        object.insert(field.clone(), json!(name));
        raw.insert(field.clone(), json!(name));
    }
    Ok(())
}

async fn commit_model(
    db: &fexu_db::driver::DatabaseConnection,
    user: &AuthUser,
    model: &dyn ManagedModel,
    id: Option<Uuid>,
    object: Map<String, Value>,
    tail: &crate::compose::SaveTail<'_>,
) -> Result<Uuid, FexuError> {
    let tx = db.begin().await.map_err(FexuError::database)?;
    let saved = async {
        let scope = access_scope(model, user, true);
        let saved = if let Some(id) = id {
            let before = model.read_in(&tx, id, &scope).await?;
            let after = model
                .update_in(&tx, id, Value::Object(object), &scope, user)
                .await?;
            audit_saved(&tx, user, model, "update", before.as_ref(), &after).await?;
            id
        } else {
            let after = model.create_in(&tx, Value::Object(object), user).await?;
            let saved = after
                .get("id")
                .and_then(Value::as_str)
                .and_then(|value| Uuid::parse_str(value).ok())
                .ok_or(FexuError::NotFound)?;
            audit_saved(&tx, user, model, "create", None, &after).await?;
            saved
        };
        crate::compose::write_tail(&tx, model.meta().key, saved, tail).await?;
        Ok(saved)
    }
    .await;
    fexu_db::conclude(tx, saved, FexuError::database).await
}

async fn audit_saved(
    tx: &fexu_db::driver::DatabaseTransaction,
    user: &AuthUser,
    model: &dyn ManagedModel,
    action: &str,
    before: Option<&Value>,
    after: &Value,
) -> Result<(), FexuError> {
    audit::insert(
        tx,
        &Event {
            actor_id: user.id,
            model_key: model.meta().key.to_owned(),
            record_id: after
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            action: action.to_owned(),
            outcome: "ok".to_owned(),
            summary: audit::change_summary(model.meta(), before, Some(after)),
            operation_id: None,
        },
    )
    .await
}

async fn record_file(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    Path((model_key, id, field)): Path<(String, Uuid, String)>,
) -> Response {
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let user = match require_get(
        &state,
        &headers,
        &registry,
        &format!("/admin/models/{model_key}/{id}"),
    )
    .await
    {
        Ok(user) => user,
        Err(response) => return response,
    };
    let allowed = registry.widgets().iter().any(|widget| {
        widget.model_key == model_key
            && widget.field == field
            && matches!(widget.kind, crate::widgets::WidgetKind::Image)
    });
    let Some(model) = registry.find(&model_key) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !allowed
        || gate(&state, &registry, &user, model.as_ref(), "read")
            .await
            .is_err()
        || read_visible(&state, &user, model.as_ref(), id, "read")
            .await
            .is_none()
    {
        let _ = locale;
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some((kind, bytes)) = crate::files::load(&state.db, &model_key, id, &field)
        .await
        .ok()
        .flatten()
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let content_type = match kind.as_str() {
        "image/png" => "image/png",
        "image/jpeg" => "image/jpeg",
        "image/gif" => "image/gif",
        "image/webp" => "image/webp",
        "image/x-icon" => "image/x-icon",
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    let mut response = Response::new(axum::body::Body::from(bytes));
    *response.status_mut() = StatusCode::OK;
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=3600"),
    );
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

async fn save_form(
    state: &AppState,
    headers: &HeaderMap,
    registry: &AdminRegistry,
    model_key: &str,
    id: Option<Uuid>,
    body: &Bytes,
) -> Response {
    let locale = locale_of(
        headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let user = match require_post(state, headers, body).await {
        Ok(user) => user,
        Err(_) => {
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
            .await
        }
    };
    let Some(model) = registry.find(model_key) else {
        return error_page(
            state,
            headers,
            registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    };
    let action_name = if id.is_some() { "update" } else { "create" };
    if gate(state, registry, &user, model.as_ref(), action_name)
        .await
        .is_err()
    {
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
    let PostedModel {
        fields: form,
        repeated,
        uploads,
    } = match read_model_body(headers, body) {
        Ok(parsed) => parsed,
        Err(()) => {
            return error_page(
                state,
                headers,
                registry,
                &locale,
                StatusCode::UNPROCESSABLE_ENTITY,
                "forbidden",
                true,
                &user.email,
            )
            .await
        }
    };
    let (mut object, mut raw) = payload(model.meta(), model.columns(), &form);
    if let Err(message) =
        apply_image_uploads(registry, model.meta().key, &mut object, &mut raw, &uploads)
    {
        let text = validation_text(&locale, &message);
        raw.insert("__error".into(), json!({ "field": validation_field(registry.field_slice(model.meta()), &message), "text": text }));
        return form_page(
            state,
            headers,
            registry,
            model_key,
            id,
            raw,
            FormFeedback {
                notice: "",
                alert: &text,
            },
        )
        .await;
    }
    let posted_links = posted_link_values(&repeated);
    if !posted_links.is_empty() {
        raw.insert("_m2m".into(), Value::Object(posted_links));
    }
    if let Err((field, code)) = normalize_temporal(model.columns(), &mut object) {
        let text = validation_text(&locale, code);
        raw.insert("__error".into(), json!({ "field": field, "text": text }));
        return form_page(
            state,
            headers,
            registry,
            model_key,
            id,
            raw,
            FormFeedback {
                notice: "",
                alert: &text,
            },
        )
        .await;
    }
    if let Err(message) = reject_locked(registry.field_slice(model.meta()), &object, id.is_none())
        .and_then(|()| crate::widgets::enforce(registry.widgets(), model.meta().key, &object))
    {
        let text = validation_text(&locale, &message);
        return form_page(
            state,
            headers,
            registry,
            model_key,
            id,
            raw,
            FormFeedback {
                notice: "",
                alert: &text,
            },
        )
        .await;
    }
    let save = form.get("save").map(String::as_str).unwrap_or("detail");
    let formed = model.meta().inlines.iter().any(|item| item.form);
    if formed && save == "rows" {
        return match crate::compose::store_extra(&state.db, &user, model.as_ref(), id, &form).await
        {
            Ok(draft) => {
                let target = match id {
                    Some(id) => format!("/admin/models/{model_key}/{id}?draft={draft}"),
                    None => format!("/admin/models/{model_key}/new?draft={draft}"),
                };
                redirect(&target)
            }
            Err(_) => {
                error_page(
                    state,
                    headers,
                    registry,
                    &locale,
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "inline_limit",
                    true,
                    &user.email,
                )
                .await
            }
        };
    }
    if !matches!(save, "detail" | "edit" | "new" | "list") {
        return error_page(
            state,
            headers,
            registry,
            &locale,
            StatusCode::UNPROCESSABLE_ENTITY,
            "forbidden",
            true,
            &user.email,
        )
        .await;
    }
    if let Err(field) = ensure_relations(state, registry, &user, model.columns(), &object).await {
        let text = catalog(&locale)
            .get("invalid_relation")
            .and_then(Value::as_str)
            .unwrap_or("Choose a visible record.")
            .to_owned();
        raw.insert("__error".into(), json!({ "field": field, "text": text }));
        return form_page(
            state,
            headers,
            registry,
            model_key,
            id,
            raw,
            FormFeedback {
                notice: "",
                alert: &text,
            },
        )
        .await;
    }
    let planned =
        match plan_memberships(state, registry, &user, model.as_ref(), id, &repeated).await {
            Ok(planned) => planned,
            Err((name, code)) => {
                let text = validation_text(&locale, code);
                raw.insert("_m2m_error".into(), json!({ name: text }));
                return form_page(
                    state,
                    headers,
                    registry,
                    model_key,
                    id,
                    raw,
                    FormFeedback {
                        notice: "",
                        alert: &text,
                    },
                )
                .await;
            }
        };
    let links: Vec<crate::compose::StoredLink<'_>> = planned
        .iter()
        .filter(|item| item.changed)
        .map(|item| crate::compose::StoredLink {
            name: &item.name,
            link: item.link,
            children: &item.next,
        })
        .collect();
    let mut files = Vec::new();
    for (field, upload) in &uploads {
        let Some(kind) = crate::files::image_type(&upload.bytes) else {
            continue;
        };
        files.push(crate::compose::StoredFile {
            field,
            kind,
            bytes: upload.bytes.as_slice(),
        });
    }
    if files.len() != uploads.len()
        || (!files.is_empty() && crate::files::ensure(&state.db).await.is_err())
    {
        return error_page(
            state,
            headers,
            registry,
            &locale,
            StatusCode::UNPROCESSABLE_ENTITY,
            "forbidden",
            true,
            &user.email,
        )
        .await;
    }
    let draft = form
        .get("_draft")
        .and_then(|value| Uuid::parse_str(value).ok())
        .map(|token| (token, user.session_id, user.id));
    let tail = crate::compose::SaveTail {
        links: &links,
        files: &files,
        draft,
    };
    if formed {
        return match crate::compose::save_with(
            &state.db,
            state.auth.as_ref().unwrap(),
            &user,
            model.as_ref(),
            |key| registry.find(key).cloned(),
            id,
            object,
            &form,
            tail,
        )
        .await
        {
            crate::compose::SaveResult::Saved(saved) => {
                let notice = if id.is_some() { "saved" } else { "created" };
                let target = match save {
                    "list" => format!("/admin/models/{model_key}?notice={notice}"),
                    "edit" => format!("/admin/models/{model_key}/{saved}?notice={notice}"),
                    "new" => format!("/admin/models/{model_key}/new?notice={notice}"),
                    _ => format!("/admin/models/{model_key}/{saved}?notice={notice}"),
                };
                redirect(&target)
            }
            crate::compose::SaveResult::Invalid(values) => {
                let text = validation_text(
                    &locale,
                    values
                        .get("__error")
                        .and_then(|value| value.get("text"))
                        .and_then(|value| value.as_str())
                        .unwrap_or("validation.inline_failed"),
                );
                let mut response = form_page(
                    state,
                    headers,
                    registry,
                    model_key,
                    id,
                    values,
                    FormFeedback {
                        notice: "",
                        alert: &text,
                    },
                )
                .await;
                *response.status_mut() = StatusCode::UNPROCESSABLE_ENTITY;
                response
            }
            crate::compose::SaveResult::Conflict(values) => {
                let text = validation_text(&locale, "conflict.stale");
                let mut response = form_page(
                    state,
                    headers,
                    registry,
                    model_key,
                    id,
                    values,
                    FormFeedback {
                        notice: "",
                        alert: &text,
                    },
                )
                .await;
                *response.status_mut() = StatusCode::CONFLICT;
                response
            }
            crate::compose::SaveResult::Missing => {
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
            crate::compose::SaveResult::Denied => {
                error_page(
                    state,
                    headers,
                    registry,
                    &locale,
                    StatusCode::FORBIDDEN,
                    "forbidden",
                    true,
                    &user.email,
                )
                .await
            }
        };
    }
    let result = if let Some(id) = id {
        if read_visible(state, &user, model.as_ref(), id, "update")
            .await
            .is_none()
        {
            return error_page(
                state,
                headers,
                registry,
                &locale,
                StatusCode::NOT_FOUND,
                "not_found",
                true,
                &user.email,
            )
            .await;
        }
        commit_model(&state.db, &user, model.as_ref(), Some(id), object, &tail).await
    } else {
        commit_model(&state.db, &user, model.as_ref(), None, object, &tail).await
    };
    match result {
        Ok(saved) => {
            let notice = if id.is_some() { "saved" } else { "created" };
            let target = match save {
                "list" => format!("/admin/models/{model_key}?notice={notice}"),
                "edit" => format!("/admin/models/{model_key}/{saved}?notice={notice}"),
                "new" => format!("/admin/models/{model_key}/new?notice={notice}"),
                _ => format!("/admin/models/{model_key}/{saved}?notice={notice}"),
            };
            redirect(&target)
        }
        Err(FexuError::Validation(message)) => {
            let field = validation_field(registry.field_slice(model.meta()), &message);
            raw.insert(
                "__error".into(),
                json!({ "field": field, "text": validation_text(&locale, &message) }),
            );
            let text = validation_text(&locale, &message);
            form_page(
                state,
                headers,
                registry,
                model_key,
                id,
                raw,
                FormFeedback {
                    notice: "",
                    alert: &text,
                },
            )
            .await
        }
        Err(FexuError::NotFound) => {
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
        Err(_) => {
            error_page(
                state,
                headers,
                registry,
                &locale,
                StatusCode::FORBIDDEN,
                "forbidden",
                true,
                &user.email,
            )
            .await
        }
    }
}

async fn load_for_form(
    state: &AppState,
    headers: &HeaderMap,
    registry: &AdminRegistry,
    model_key: &str,
    id: Uuid,
) -> Option<Map<String, Value>> {
    let user = require_get(state, headers, registry, "/admin").await.ok()?;
    let model = registry.find(model_key)?;
    let mut record = read_visible(state, &user, model.as_ref(), id, "update").await?;
    redact(model.meta(), &mut record);
    record.as_object().cloned()
}

async fn read_visible(
    state: &AppState,
    user: &AuthUser,
    model: &dyn ManagedModel,
    id: Uuid,
    action: &str,
) -> Option<Value> {
    if list_blocked(model) {
        return None;
    }
    let scope = access_scope(model, user, action != "read");
    let value = model.read(&state.db, id, &scope).await.ok()??;
    if model.policy_bound()
        && model.descriptor().requires_object_policy
        && !user.is_superuser
        && !model.object_allows(user, action, &value)
    {
        return None;
    }
    Some(value)
}

async fn input_fields(
    locale: &str,
    registry: &AdminRegistry,
    state: &AppState,
    user: &AuthUser,
    model: &dyn ManagedModel,
    values: &Map<String, Value>,
) -> Vec<Value> {
    let error_field = values
        .get("__error")
        .and_then(|value| value.get("field"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let error_text = values
        .get("__error")
        .and_then(|value| value.get("text"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let hint_select = catalog(locale)
        .get("hint_select")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let mut fields = Vec::new();
    for field in registry.field_slice(model.meta()) {
        if field.hidden || sensitive(field.name) {
            continue;
        }
        let Some(column) = model
            .columns()
            .iter()
            .find(|column| column.name == field.name)
        else {
            continue;
        };
        let current = values.get(field.name).cloned().unwrap_or(Value::Null);
        let locked = values.get("_lock").and_then(Value::as_str) == Some(field.name);
        if field.readonly || field.name == "id" || locked {
            if field.name == "id" && !values.contains_key(field.name) {
                continue;
            }
            fields.push(json!({
                "name": field.name,
                "label": field.label,
                "fieldset": field.fieldset,
                "kind": "readonly",
                "required": false,
                "invalid": false,
                "error": "",
                "hint": "",
                "pick": "",
                "value": display_cell(locale, registry.branding().currency.as_str(), field.name, &current),
                "raw": match &current {
                    Value::String(text) => text.clone(),
                    _ => String::new(),
                },
                "locked": locked,
                "checked": false
            }));
            continue;
        }
        let mut input = json!({
            "name": field.name,
            "label": field.label,
            "fieldset": field.fieldset,
            "pick": if column.foreign_table.is_some() { pick_path(model.meta().key, values, field.name) } else { String::new() },
            "required": !column.nullable && column.default_bool.is_none(),
            "invalid": error_field == field.name,
            "error": if error_field == field.name { error_text } else { "" },
            "hint": if column.foreign_table.is_some() { hint_select.clone() } else { String::new() },
            "owns_relation": column.foreign_table.is_some(),
            "value": match &current {
                Value::String(text) => text.clone(),
                Value::Number(number) => number.to_string(),
                Value::Bool(flag) => flag.to_string(),
                _ => String::new(),
            },
            "checked": match &current {
                Value::Bool(flag) => *flag,
                Value::String(text) => text == "true" || text == "1",
                Value::Null => column.default_bool.unwrap_or(false),
                _ => false,
            }
        });
        if let Some(widget) = registry
            .widgets()
            .iter()
            .find(|widget| widget.model_key == model.meta().key && widget.field == field.name)
        {
            if matches!(widget.kind, crate::widgets::WidgetKind::Readonly) {
                fields.push(json!({
                    "name": field.name,
                    "label": field.label,
                    "fieldset": field.fieldset,
                    "kind": "readonly",
                    "required": false,
                    "invalid": false,
                    "error": "",
                    "hint": "",
                    "pick": "",
                    "value": display_cell(locale, registry.branding().currency.as_str(), field.name, &current),
                    "raw": "",
                    "locked": false,
                    "checked": false
                }));
                continue;
            }
            let (kind, input_type) = crate::widgets::presentation(widget.kind);
            input["kind"] = json!(kind);
            input["input"] = json!(input_type);
            if let crate::widgets::WidgetKind::Textarea { rows, .. } = widget.kind {
                input["rows"] = json!(rows);
            }
            if matches!(widget.kind, crate::widgets::WidgetKind::Image) {
                let preview = match values
                    .get("id")
                    .and_then(Value::as_str)
                    .and_then(|value| Uuid::parse_str(value).ok())
                {
                    Some(id)
                        if crate::files::exists(&state.db, model.meta().key, id, field.name)
                            .await =>
                    {
                        format!(
                            "/admin/models/{}/{id}/file/{}",
                            model.meta().key,
                            field.name
                        )
                    }
                    _ => String::new(),
                };
                if !preview.is_empty() || !input["value"].as_str().unwrap_or("").is_empty() {
                    input["required"] = json!(false);
                }
                input["preview"] = json!(preview);
                input["hint"] = json!(catalog(locale)
                    .get("hint_image")
                    .and_then(Value::as_str)
                    .unwrap_or(""));
            }
        } else if column.foreign_table.is_some() {
            input["kind"] = json!("select");
            let (choices, truncated) = relation_options(
                registry,
                state,
                user,
                column,
                current.as_str().unwrap_or(""),
            )
            .await;
            input["options"] = choices;
            input["truncated"] = json!(truncated);
            // Retain an unavailable current relation instead of silently selecting NULL.
            if let Some(id) = current.as_str().filter(|id| !id.is_empty()) {
                if let Some(options) = input["options"].as_array_mut() {
                    if !options.iter().any(|option| option["selected"] == true) {
                        options.insert(0, json!({
                            "id": id, "selected": true,
                            "label": catalog(locale)[if user.is_superuser { "missing_relation" } else { "hidden_relation" }]
                        }));
                    }
                }
            }
            input["input"] = json!("text");
            input["related"] = related_links(
                state,
                registry,
                user,
                column,
                current.as_str().unwrap_or(""),
            )
            .await;
        } else if column.kind == ColumnKind::Choice {
            input["kind"] = json!("select");
            input["options"] = json!(column
                .choices
                .iter()
                .map(|choice| json!({
                    "id": choice.value,
                    "label": if locale == "de" { choice.label_de } else { choice.label_en },
                    "selected": current.as_str() == Some(choice.value),
                }))
                .collect::<Vec<_>>());
            input["input"] = json!("text");
        } else if column.kind == ColumnKind::Boolean {
            input["kind"] = json!("boolean");
        } else {
            input["kind"] = json!("text");
            input["input"] = json!(match column.kind {
                ColumnKind::BigInt => "number",
                ColumnKind::Date => "date",
                ColumnKind::Time => "time",
                ColumnKind::DateTime => "datetime-local",
                ColumnKind::Uuid
                | ColumnKind::Text
                | ColumnKind::Boolean
                | ColumnKind::Choice
                | ColumnKind::Json => "text",
                ColumnKind::Decimal => "text",
            });
            if column.kind == ColumnKind::DateTime {
                if let Some(text) = input["value"].as_str() {
                    input["value"] = json!(fexu_core::temporal::datetime_local_value(text));
                }
                input["hint"] = json!(catalog(locale)
                    .get("utc_hint")
                    .and_then(Value::as_str)
                    .unwrap_or("UTC"));
            }
        }
        fields.push(input);
    }
    fields
}

async fn relation_options(
    registry: &AdminRegistry,
    state: &AppState,
    user: &AuthUser,
    column: &ColumnSpec,
    current: &str,
) -> (Value, bool) {
    let Some(table) = column.foreign_table else {
        return (json!([]), false);
    };
    let Some(target) = registry.model_for_table(table) else {
        return (json!([]), false);
    };
    if list_blocked(target.as_ref())
        || permit(state.auth.as_ref().unwrap(), user, target.meta(), "read")
            .await
            .is_err()
    {
        return (json!([]), false);
    }
    let Some(label) = display_column(registry.field_slice(target.meta()), target.columns()) else {
        return (json!([]), false);
    };
    let scope = access_scope(target.as_ref(), user, false);
    let Ok((rows, _, total)) = target
        .page(&state.db, &[], None, label, false, &[], &[], 0, 100, &scope)
        .await
    else {
        return (json!([]), false);
    };
    let truncated = total > rows.len() as i64;
    let mut options: Vec<Value> = rows
        .into_iter()
        .filter_map(|row| {
            let id = row.get("id")?.as_str()?;
            Some(json!({
                "id": id,
                "label": row.get(label)?.as_str().unwrap_or(""),
                "selected": id == current
            }))
        })
        .collect();
    if !current.is_empty()
        && !options
            .iter()
            .any(|option| option.get("id").and_then(Value::as_str) == Some(current))
    {
        if let Ok(id) = Uuid::parse_str(current) {
            if let Some(record) = read_visible(state, user, target.as_ref(), id, "read").await {
                options.insert(
                    0,
                    json!({
                        "id": current,
                        "label": record.get(label).and_then(Value::as_str).unwrap_or(""),
                        "selected": true
                    }),
                );
            }
        }
    }
    (Value::Array(options), truncated)
}

fn pick_path(model_key: &str, values: &Map<String, Value>, _field: &str) -> String {
    match values
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    {
        Some(id) => format!("/admin/models/{model_key}/{id}/draft"),
        None => format!("/admin/models/{model_key}/draft"),
    }
}

fn apply_picks(
    meta: &fexu_core::AdminModelMeta,
    columns: &[ColumnSpec],
    params: &HashMap<String, String>,
    values: &mut Map<String, Value>,
) {
    for field in meta.fields {
        if field.hidden || field.readonly {
            continue;
        }
        let Some(column) = columns.iter().find(|column| column.name == field.name) else {
            continue;
        };
        if column.foreign_table.is_none() {
            continue;
        }
        let Some(value) = params.get(field.name) else {
            continue;
        };
        if Uuid::parse_str(value).is_ok() {
            values.insert(field.name.to_owned(), json!(value));
        }
    }
}

fn form_rows(group: &[&Value], layout: Option<&[Vec<&'static str>]>) -> Vec<Value> {
    let Some(layout) = layout else {
        return group
            .iter()
            .map(|field| json!({ "fields": [*field] }))
            .collect();
    };
    layout
        .iter()
        .map(|row| {
            let cells: Vec<&Value> = row
                .iter()
                .filter_map(|name| {
                    group
                        .iter()
                        .copied()
                        .find(|field| field.get("name").and_then(Value::as_str) == Some(*name))
                })
                .collect();
            json!({ "fields": cells })
        })
        .filter(|row| {
            row.get("fields")
                .and_then(Value::as_array)
                .is_some_and(|cells| !cells.is_empty())
        })
        .collect()
}

fn owns_relation(field: &Value) -> bool {
    field
        .get("owns_relation")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn form_sections(
    fieldsets: &[fexu_core::AdminFieldset],
    fields: &[Value],
    layouts: Option<&[crate::configure::FormLayout]>,
) -> Vec<Value> {
    let content: Vec<&Value> = fields
        .iter()
        .filter(|field| !owns_relation(field))
        .collect();
    let mut sections = if fieldsets.is_empty() {
        if content.is_empty() {
            Vec::new()
        } else {
            vec![json!({
                "name": "",
                "description": "",
                "optional": false,
                "fields": content,
                "rows": form_rows(&content, None)
            })]
        }
    } else {
        fieldsets
            .iter()
            .filter_map(|section| {
                let group: Vec<&Value> = content
                    .iter()
                    .copied()
                    .filter(|field| {
                        field.get("fieldset").and_then(Value::as_str) == Some(section.name)
                    })
                    .collect();
                if group.is_empty() {
                    return None;
                }
                let layout = layouts.and_then(|items| {
                    items
                        .iter()
                        .find(|item| item.name == section.name)
                        .map(|item| item.rows.as_slice())
                });
                Some(json!({
                    "name": section.name,
                    "description": section.description,
                    "optional": section.optional,
                    "fields": group,
                    "rows": form_rows(&group, layout)
                }))
            })
            .collect()
    };
    for field in fields.iter().filter(|field| owns_relation(field)) {
        let label = field.get("label").and_then(Value::as_str).unwrap_or("");
        let group = [field];
        sections.push(json!({
            "name": label,
            "description": "",
            "optional": false,
            "fields": [field],
            "rows": form_rows(&group, None)
        }));
    }
    sections
}

async fn related_links(
    state: &AppState,
    registry: &AdminRegistry,
    user: &AuthUser,
    column: &ColumnSpec,
    current: &str,
) -> Value {
    let Some(table) = column.foreign_table else {
        return Value::Null;
    };
    let Some(target) = registry.model_for_table(table) else {
        return Value::Null;
    };
    let auth = state.auth.as_ref().unwrap();
    let key = target.meta().key;
    json!({
        "key": key,
        "id": current,
        "can_view": permit(auth, user, target.meta(), "read").await.is_ok(),
        "can_change": permit(auth, user, target.meta(), "update").await.is_ok(),
        "can_add": permit(auth, user, target.meta(), "create").await.is_ok(),
        "can_delete": permit(auth, user, target.meta(), "delete").await.is_ok()
    })
}

async fn ensure_relations(
    state: &AppState,
    registry: &AdminRegistry,
    user: &AuthUser,
    columns: &[ColumnSpec],
    object: &Map<String, Value>,
) -> Result<(), String> {
    for column in columns {
        let Some(table) = column.foreign_table else {
            continue;
        };
        let Some(value) = object.get(column.name) else {
            continue;
        };
        if value.is_null() {
            continue;
        }
        let Some(text) = value.as_str() else {
            return Err(column.name.to_owned());
        };
        if text.is_empty() {
            continue;
        }
        let Ok(id) = Uuid::parse_str(text) else {
            return Err(column.name.to_owned());
        };
        let Some(target) = registry.model_for_table(table) else {
            if registry
                .models()
                .iter()
                .filter(|candidate| candidate.descriptor().table == table)
                .count()
                > 1
            {
                return Err(column.name.to_owned());
            }
            continue;
        };
        if read_visible(state, user, target.as_ref(), id, "read")
            .await
            .is_none()
        {
            return Err(column.name.to_owned());
        }
    }
    Ok(())
}

fn payload(
    meta: &fexu_core::AdminModelMeta,
    columns: &[ColumnSpec],
    form: &HashMap<String, String>,
) -> (Map<String, Value>, Map<String, Value>) {
    let mut object = Map::new();
    let mut raw = Map::new();
    for (key, value) in form {
        if key == "csrf_token"
            || key == "next"
            || key == "save"
            || key == "pick_field"
            || key == "_action"
            || key == "_lock"
            || key == "_use_draft"
            || key == "_picks"
        {
            continue;
        }
        if key == "m2m_present" || key.starts_with("m2m.") {
            continue;
        }
        if key == "_draft" {
            raw.insert(key.clone(), json!(value));
            continue;
        }
        if key.starts_with("inline_") || key.starts_with("extra_") || key == "basis" {
            raw.insert(key.clone(), json!(value));
            continue;
        }
        raw.insert(key.clone(), json!(value));
        let Some(field) = meta.fields.iter().find(|field| field.name == key) else {
            object.insert(key.clone(), json!(value));
            continue;
        };
        let Some(column) = columns.iter().find(|column| column.name == field.name) else {
            continue;
        };
        object.insert(key.clone(), typed_value(column, value));
    }
    (object, raw)
}

fn typed_value(column: &ColumnSpec, value: &str) -> Value {
    if value.is_empty() && column.nullable {
        return Value::Null;
    }
    match column.kind {
        ColumnKind::Boolean => json!(matches!(value, "true" | "1" | "on")),
        ColumnKind::BigInt => value
            .parse::<i64>()
            .map(|number| json!(number))
            .unwrap_or_else(|_| json!(value)),
        _ => json!(value),
    }
}

fn visible_fields(
    locale: &str,
    currency: &str,
    meta: &fexu_core::AdminModelMeta,
    columns: &[ColumnSpec],
    record: &Value,
    relations: &HashMap<String, RelationLabels>,
    superuser: bool,
) -> Vec<Value> {
    meta.fields
        .iter()
        .filter(|field| !field.hidden && !sensitive(field.name))
        .map(|field| {
            let mut lead = false;
            let cell = present_cell(
                locale,
                currency,
                columns,
                field.name,
                record.get(field.name).unwrap_or(&Value::Null),
                relations,
                superuser,
                &mut lead,
            );
            json!({
            "label": field.label,
            "readonly": field.readonly || field.name == "id",
                    "value": cell["text"].as_str().unwrap_or(""),
                    "relation": cell["relation"].as_bool().unwrap_or(false),
                    "href": cell["href"].as_str().unwrap_or(""),
                    "ref": cell["ref"].as_str().unwrap_or(""),
                    "short": cell["short"].as_str().unwrap_or("")
                })
        })
        .collect()
}

fn resolved_sort(
    fields: &[fexu_core::AdminFieldMeta],
    ordering: Option<&str>,
    params: &HashMap<String, String>,
) -> Result<(String, bool), ()> {
    if let Some(sort) = params.get("sort").filter(|value| !value.is_empty()) {
        if fields.iter().any(|field| {
            field.name == sort && !field.hidden && (field.sortable || field.name == "id")
        }) {
            Ok((
                sort.clone(),
                params.get("dir").map(String::as_str) == Some("desc"),
            ))
        } else {
            Err(())
        }
    } else if let Some(ordering) = ordering {
        Ok((
            ordering.trim_start_matches('-').to_owned(),
            ordering.starts_with('-'),
        ))
    } else {
        Ok((
            "id".to_owned(),
            params.get("dir").map(String::as_str) == Some("desc"),
        ))
    }
}

fn link_names(
    fields: &[fexu_core::AdminFieldMeta],
    specs: &[ColumnSpec],
    order: Option<&[&str]>,
) -> Vec<&'static str> {
    let listed = listed_fields(fields, specs, order);
    let foreign = |name: &str| {
        specs
            .iter()
            .any(|column| column.name == name && column.foreign_table.is_some())
    };
    let chosen: Vec<&'static str> = listed
        .iter()
        .filter(|field| field.list_link && !foreign(field.name))
        .map(|field| field.name)
        .collect();
    if !chosen.is_empty() {
        return chosen;
    }
    let text = |name: &str| {
        specs
            .iter()
            .any(|column| column.name == name && column.kind == ColumnKind::Text)
    };
    listed
        .iter()
        .find(|field| !foreign(field.name) && text(field.name))
        .or_else(|| listed.iter().find(|field| !foreign(field.name)))
        .or_else(|| listed.first())
        .map(|field| vec![field.name])
        .unwrap_or_default()
}

fn list_columns(
    fields: &[fexu_core::AdminFieldMeta],
    specs: &[ColumnSpec],
    model: &str,
    params: &HashMap<String, String>,
    sort: &str,
    descending: bool,
    order: Option<&[&str]>,
) -> Vec<Value> {
    let dir = if descending { "desc" } else { "asc" };
    listed_fields(fields, specs, order)
        .into_iter()
        .map(|field| {
            let next_dir = if sort == field.name && dir != "desc" {
                "desc"
            } else {
                "asc"
            };
            let sortable = field.sortable || field.name == "id";
            let active = sortable && sort == field.name;
            json!({
                "name": field.name,
                "label": field.label,
                "readonly": field.readonly && field.name != "id",
                "sortable": sortable,
                "sorted": active,
                "sort_state": if active { if dir == "desc" { "descending" } else { "ascending" } } else { "" },
                "mark": if active { if dir == "desc" { "↓" } else { "↑" } } else { "" },
                "sort_href": format!("/admin/models/{model}?{}", sort_query(params, field.name, next_dir))
            })
        })
        .collect()
}

/// Columns displayed in a model list.
///
/// Fields marked with `list_display` follow `ModelAdmin::list_display` order;
/// otherwise, model field order applies. The `id` column is excluded:
/// it contains a UUID and is not useful for reading the table.
///
/// Without explicitly listed fields, use the first visible text field, then another visible field.
/// Exclude `hidden` fields and sensitive names.
fn listed_fields<'a>(
    fields: &'a [fexu_core::AdminFieldMeta],
    specs: &[ColumnSpec],
    order: Option<&[&str]>,
) -> Vec<&'a fexu_core::AdminFieldMeta> {
    let visible = |field: &fexu_core::AdminFieldMeta| {
        field.name != "id" && !field.hidden && !sensitive(field.name)
    };
    let mut chosen: Vec<_> = fields
        .iter()
        .filter(|field| field.list_display && visible(field))
        .collect();
    if chosen.is_empty() {
        let text = fields.iter().find(|field| {
            visible(field) && column_kind(specs, field.name) == Some(ColumnKind::Text)
        });
        let other = fields
            .iter()
            .find(|field| visible(field) && column_kind(specs, field.name).is_some());
        if let Some(field) = text.or(other) {
            chosen.push(field);
        }
    }
    apply_column_order(&mut chosen, order);
    chosen
}

fn apply_column_order(fields: &mut [&fexu_core::AdminFieldMeta], order: Option<&[&str]>) {
    let Some(order) = order else {
        return;
    };
    fields.sort_by_key(|field| {
        order
            .iter()
            .position(|name| *name == field.name)
            .unwrap_or(usize::MAX)
    });
}

fn column_kind(specs: &[ColumnSpec], name: &str) -> Option<ColumnKind> {
    specs
        .iter()
        .find(|column| column.name == name)
        .map(|column| column.kind)
}

fn list_href(params: &HashMap<String, String>, key: &str, value: Option<&str>) -> String {
    let mut pairs: Vec<(String, String)> = params
        .iter()
        .filter(|(name, current)| {
            !current.is_empty()
                && !matches!(name.as_str(), "page" | "notice" | "changed")
                && name.as_str() != key
        })
        .map(|(name, current)| ((*name).clone(), current.clone()))
        .collect();
    if let Some(value) = value.filter(|value| !value.is_empty()) {
        pairs.push((key.to_owned(), value.to_owned()));
    }
    pairs.sort();
    if pairs.is_empty() {
        return "?".to_owned();
    }
    format!(
        "?{}",
        pairs
            .iter()
            .map(|(name, current)| format!("{}={}", url_encode(name), url_encode(current)))
            .collect::<Vec<_>>()
            .join("&")
    )
}

fn facet_options(
    params: &HashMap<String, String>,
    name: &str,
    value: &str,
    choices: Vec<(String, String)>,
    all_label: &str,
) -> Vec<Value> {
    let key = format!("filter.{name}");
    let mut options = vec![json!({
        "id": "",
        "label": all_label,
        "href": list_href(params, &key, None),
        "current": value.is_empty()
    })];
    for (id, label) in choices {
        let href = list_href(params, &key, Some(id.as_str()));
        let current = value == id;
        options.push(json!({
            "id": id,
            "label": label,
            "href": href,
            "current": current
        }));
    }
    options
}

fn choice_rows(options: &Value) -> Vec<(String, String)> {
    let Some(rows) = options.as_array() else {
        return Vec::new();
    };
    rows.iter()
        .filter_map(|option| {
            Some((
                option.get("id")?.as_str()?.to_owned(),
                option
                    .get("label")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
            ))
        })
        .collect()
}

async fn filter_inputs(
    locale: &str,
    registry: &AdminRegistry,
    state: &AppState,
    user: &AuthUser,
    meta: &fexu_core::AdminModelMeta,
    columns: &[ColumnSpec],
    params: &HashMap<String, String>,
) -> Vec<Value> {
    let all = catalog(locale)["all"].as_str().unwrap_or("All").to_owned();
    let yes = catalog(locale)["yes"].as_str().unwrap_or("Yes").to_owned();
    let no = catalog(locale)["no"].as_str().unwrap_or("No").to_owned();
    let mut filters = Vec::new();
    for field in registry
        .field_slice(meta)
        .iter()
        .filter(|field| field.filterable && !field.hidden)
    {
        let column = columns.iter().find(|column| column.name == field.name);
        let raw = params
            .get(&format!("filter.{}", field.name))
            .cloned()
            .unwrap_or_default();
        let (kind, options) = match column {
            Some(column) if column.foreign_table.is_some() => {
                let (options, _) = relation_options(registry, state, user, column, &raw).await;
                ("uuid", options)
            }
            Some(column) if column.kind == ColumnKind::Boolean => ("boolean", json!([])),
            Some(column) if column.kind == ColumnKind::Choice => (
                "choice",
                Value::Array(
                    column
                        .choices
                        .iter()
                        .map(|choice| {
                            json!({
                                "id": choice.value,
                                "label": if locale == "de" { choice.label_de } else { choice.label_en }
                            })
                        })
                        .collect(),
                ),
            ),
            Some(column) if column.kind == ColumnKind::BigInt => ("number", json!([])),
            Some(column) if column.kind == ColumnKind::Date => ("date", json!([])),
            Some(column) if column.kind == ColumnKind::Time => ("time", json!([])),
            Some(column) if column.kind == ColumnKind::DateTime => ("datetime", json!([])),
            _ => ("text", json!([])),
        };
        let value = if kind == "boolean" {
            match raw.as_str() {
                "true" | "1" => "true".to_owned(),
                "false" | "0" => "false".to_owned(),
                _ => String::new(),
            }
        } else {
            raw
        };
        let facet = matches!(kind, "boolean" | "uuid" | "choice");
        let options = if kind == "boolean" {
            Value::Array(facet_options(
                params,
                field.name,
                &value,
                vec![
                    ("true".to_owned(), yes.clone()),
                    ("false".to_owned(), no.clone()),
                ],
                &all,
            ))
        } else if facet {
            Value::Array(facet_options(
                params,
                field.name,
                &value,
                choice_rows(&options),
                &all,
            ))
        } else {
            options
        };
        filters.push(json!({
            "name": field.name,
            "label": field.label,
            "kind": kind,
            "facet": facet,
            "value": value,
            "options": options,
            "yes": yes,
            "no": no
        }));
    }
    for filter in registry
        .filters()
        .iter()
        .filter(|filter| filter.model_key == meta.key)
    {
        let raw = params
            .get(&format!("filter.{}", filter.name))
            .cloned()
            .unwrap_or_default();
        let (kind, options) = match &filter.kind {
            crate::filters::FilterKind::Choice { options, .. } => (
                "choice",
                Value::Array(
                    options
                        .iter()
                        .map(|option| {
                            json!({
                                "id": option.value,
                                "label": if locale == "de" { option.label_de } else { option.label_en }
                            })
                        })
                        .collect(),
                ),
            ),
            crate::filters::FilterKind::Equals { .. } => ("text", json!([])),
        };
        let facet = kind == "choice";
        let options = if facet {
            Value::Array(facet_options(
                params,
                filter.name,
                &raw,
                choice_rows(&options),
                &all,
            ))
        } else {
            options
        };
        filters.push(json!({
            "name": filter.name,
            "label": filter.label(locale),
            "kind": kind,
            "facet": facet,
            "value": raw,
            "options": options,
            "yes": yes,
            "no": no
        }));
    }
    if let Some(order) = registry.filter_order_of(meta) {
        filters.sort_by(|left, right| {
            let rank = |item: &Value| {
                item.get("name")
                    .and_then(Value::as_str)
                    .and_then(|name| order.iter().position(|entry| *entry == name))
                    .unwrap_or(usize::MAX)
            };
            rank(left).cmp(&rank(right))
        });
    }
    filters
}

#[allow(clippy::too_many_arguments)]
fn present_cell(
    locale: &str,
    currency: &str,
    columns: &[ColumnSpec],
    name: &str,
    value: &Value,
    relations: &HashMap<String, RelationLabels>,
    superuser: bool,
    lead_taken: &mut bool,
) -> Value {
    let mut text = display_cell(locale, currency, name, value);
    if let Some(column) = columns
        .iter()
        .find(|column| column.name == name && column.kind == ColumnKind::Choice)
    {
        if !text.is_empty() && text != "—" {
            text = fexu_core::choice_label(column.choices, locale, &text);
        }
    }
    let foreign = columns
        .iter()
        .find(|column| column.name == name)
        .and_then(|column| column.foreign_table);
    if let Some(table) = foreign {
        if text.is_empty() || text == "—" {
            return json!({ "text": "—", "short": "", "uuid": false, "lead": false, "relation": false, "ref": "", "href": "" });
        }
        let entry = relations.get(table);
        let resolved = entry.and_then(|entry| entry.labels.get(&text));
        let shown = if let Some(label) = resolved {
            label.clone()
        } else if superuser {
            catalog(locale)["missing_relation"]
                .as_str()
                .unwrap_or("Missing")
                .to_owned()
        } else {
            catalog(locale)["hidden_relation"]
                .as_str()
                .unwrap_or("Not visible")
                .to_owned()
        };
        let href = match (entry, resolved) {
            (Some(entry), Some(_)) if !entry.model_key.is_empty() => {
                format!("/admin/models/{}/{text}", entry.model_key)
            }
            _ => String::new(),
        };
        let lead = !*lead_taken;
        *lead_taken = true;
        return json!({
            "text": shown,
            "short": shorten_uuid(&text),
            "uuid": false,
            "lead": lead,
            "relation": true,
            "ref": text,
            "href": href
        });
    }
    let uuid = is_uuid(&text);
    let lead = !uuid && !*lead_taken;
    if lead {
        *lead_taken = true;
    }
    json!({
        "text": text,
        "short": if uuid { shorten_uuid(&text) } else { text.clone() },
        "uuid": uuid,
        "lead": lead,
        "relation": false,
        "ref": "",
        "href": ""
    })
}

struct RelationLabels {
    model_key: &'static str,
    labels: HashMap<String, String>,
}

async fn relation_names(
    registry: &AdminRegistry,
    state: &AppState,
    user: &AuthUser,
    columns: &[ColumnSpec],
    rows: &[Value],
) -> Result<HashMap<String, RelationLabels>, FexuError> {
    let mut grouped: HashMap<&str, Vec<Uuid>> = HashMap::new();
    for column in columns
        .iter()
        .filter(|column| column.foreign_table.is_some())
    {
        let Some(table) = column.foreign_table else {
            continue;
        };
        for row in rows {
            let Some(text) = row.get(column.name).and_then(Value::as_str) else {
                continue;
            };
            if let Ok(id) = Uuid::parse_str(text) {
                grouped.entry(table).or_default().push(id);
            }
        }
    }
    let mut resolved = HashMap::new();
    for (table, ids) in grouped {
        let Some(target) = registry.model_for_table(table) else {
            resolved.insert(
                table.to_owned(),
                RelationLabels {
                    model_key: "",
                    labels: HashMap::new(),
                },
            );
            continue;
        };
        if list_blocked(target.as_ref())
            || permit(state.auth.as_ref().unwrap(), user, target.meta(), "read")
                .await
                .is_err()
        {
            resolved.insert(
                table.to_owned(),
                RelationLabels {
                    model_key: "",
                    labels: HashMap::new(),
                },
            );
            continue;
        }
        let Some(label) = display_column(registry.field_slice(target.meta()), target.columns())
        else {
            resolved.insert(
                table.to_owned(),
                RelationLabels {
                    model_key: "",
                    labels: HashMap::new(),
                },
            );
            continue;
        };
        let scope = access_scope(target.as_ref(), user, false);
        let pairs = query_labels(
            &state.db,
            target.descriptor().table,
            target.columns(),
            label,
            &ids,
            &scope,
        )
        .await?;
        resolved.insert(
            table.to_owned(),
            RelationLabels {
                model_key: target.meta().key,
                labels: pairs.into_iter().collect(),
            },
        );
    }
    Ok(resolved)
}

fn display_cell(locale: &str, currency: &str, name: &str, value: &Value) -> String {
    match value {
        Value::Bool(true) => catalog(locale)["yes"].as_str().unwrap_or("Yes").to_owned(),
        Value::Bool(false) => catalog(locale)["no"].as_str().unwrap_or("No").to_owned(),
        Value::Number(number) if name.ends_with("_cents") => {
            let cents = number.as_i64().unwrap_or(0);
            let sign = if cents < 0 { "-" } else { "" };
            format!(
                "{sign}{} {:.2}",
                currency,
                (cents.unsigned_abs() as f64) / 100.0
            )
        }
        Value::Number(number) => group_int(locale, number.as_i64().unwrap_or(0)),
        Value::String(text) => text.clone(),
        Value::Null => "—".to_owned(),
        other => other.to_string(),
    }
}

fn group_int(locale: &str, value: i64) -> String {
    let separator = if locale == "de" { '.' } else { ',' };
    let digits: Vec<char> = value.unsigned_abs().to_string().chars().collect();
    let mut output = String::new();
    for (index, char) in digits.iter().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            output.push(separator);
        }
        output.push(*char);
    }
    if value < 0 {
        format!("-{output}")
    } else {
        output
    }
}

fn confirmed(body: &Bytes) -> Result<(), &'static str> {
    let values = FormValues::parse_urlencoded(&String::from_utf8_lossy(body))
        .map_err(|_| "confirm_invalid")?;
    match require_confirmation(&values) {
        Ok(()) => Ok(()),
        Err(errors) if errors.for_field("confirm") == Some("validation.invalid") => {
            Err("confirm_invalid")
        }
        Err(_) => Err("confirm_required"),
    }
}

async fn role_names(auth: &fexu_auth::AuthService) -> Vec<String> {
    let mut names: Vec<String> = auth
        .list_roles()
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|(_, name)| name)
        .collect();
    names.sort();
    names
}

async fn user_new(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
) -> Response {
    user_form(&state, &headers, &registry, None, None, "", "").await
}

async fn user_create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    body: Bytes,
) -> Response {
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let actor = match require_directory_post(&state, &headers, &registry, &body).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    if let Err(code) = confirmed(&body) {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::UNPROCESSABLE_ENTITY,
            code,
            true,
            &actor.email,
        )
        .await;
    }
    let values = match FormValues::parse_urlencoded(&String::from_utf8_lossy(&body)) {
        Ok(values) => values,
        Err(_) => {
            return error_page(
                &state,
                &headers,
                &registry,
                &locale,
                StatusCode::UNPROCESSABLE_ENTITY,
                "confirm_invalid",
                true,
                &actor.email,
            )
            .await
        }
    };
    if reject_unknown(
        &values,
        &[
            "confirm",
            "email",
            "name",
            "password",
            "password_confirm",
            "active",
            "staff",
            "roles",
        ],
    )
    .is_err()
    {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::UNPROCESSABLE_ENTITY,
            "forbidden",
            true,
            &actor.email,
        )
        .await;
    }
    let email = required_text(
        &values,
        TextSpec {
            name: "email",
            required: true,
            min_length: None,
            max_length: Some(254),
            email: true,
            url: false,
        },
    );
    let active = required_bool(&values, "active");
    let staff = required_bool(&values, "staff");
    let roles = repeated(&body, "roles");
    let Some(password) = values.get("password") else {
        let text = catalog(&locale)["password_invalid"]
            .as_str()
            .unwrap_or("The password must contain 12 to 128 characters.")
            .to_owned();
        let mut draft = user_draft(&values, Some(roles.clone()));
        draft.password_error = text.clone();
        return user_form(&state, &headers, &registry, None, Some(draft), &text, "").await;
    };
    let password_confirm = values.get("password_confirm");
    if email.is_err() || active.is_err() || staff.is_err() || password_confirm != Some(password) {
        let copy = catalog(&locale);
        let message = if password_confirm != Some(password) {
            copy["password_mismatch"]
                .as_str()
                .unwrap_or("The passwords do not match.")
        } else if email.is_err() {
            copy["invalid_credentials"]
                .as_str()
                .unwrap_or("Invalid email or password.")
        } else {
            copy["confirm_invalid"]
                .as_str()
                .unwrap_or("The confirmation is invalid.")
        };
        let mut draft = user_draft(&values, Some(roles));
        if email.is_err() {
            draft.email_error = message.to_owned();
        }
        if password_confirm != Some(password) {
            draft.password_error = message.to_owned();
        }
        return user_form(&state, &headers, &registry, None, Some(draft), message, "").await;
    }
    let auth = state.auth.as_ref().unwrap();
    let known = role_names(auth).await;
    if roles
        .iter()
        .any(|role| !known.iter().any(|known| known == role))
    {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::UNPROCESSABLE_ENTITY,
            "unknown_role",
            true,
            &actor.email,
        )
        .await;
    }
    let created = crate::directory::create_user(
        auth,
        actor.id,
        email.as_ref().unwrap(),
        password,
        active.unwrap(),
        staff.unwrap(),
        &roles,
    )
    .await;
    let created = match created {
        Ok(user) => user,
        Err(fexu_core::FexuError::Validation(message)) if message.contains("password") => {
            let text = catalog(&locale)["password_invalid"]
                .as_str()
                .unwrap_or("The password must contain 12 to 128 characters.")
                .to_owned();
            let mut draft = user_draft(&values, Some(roles.clone()));
            draft.password_error = text.clone();
            return user_form(&state, &headers, &registry, None, Some(draft), &text, "").await;
        }
        Err(fexu_core::FexuError::Validation(_)) => {
            let text = catalog(&locale)["email_taken"]
                .as_str()
                .unwrap_or("That email is already registered.")
                .to_owned();
            let mut draft = user_draft(&values, Some(roles.clone()));
            draft.email_error = text.clone();
            return user_form(&state, &headers, &registry, None, Some(draft), &text, "").await;
        }
        Err(_) => {
            return error_page(
                &state,
                &headers,
                &registry,
                &locale,
                StatusCode::UNPROCESSABLE_ENTITY,
                "forbidden",
                true,
                &actor.email,
            )
            .await
        }
    };
    let name = values.get("name").unwrap_or("");
    if auth.set_display_name(created.id, name).await.is_err() {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::UNPROCESSABLE_ENTITY,
            "forbidden",
            true,
            &actor.email,
        )
        .await;
    }
    redirect(&format!(
        "/admin/directory/users/{}?notice=created",
        created.id
    ))
}

async fn user_detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    Path(id): Path<Uuid>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let actor = match require_roles(&state, &headers, &registry, "/admin/directory/users").await {
        Ok(user) => user,
        Err(response) => return response,
    };
    let auth = state.auth.as_ref().unwrap();
    let Some(target) = auth.get_user(id).await.ok().flatten() else {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &actor.email,
        )
        .await;
    };
    if actor.is_superuser {
        return user_form(
            &state,
            &headers,
            &registry,
            Some(id),
            None,
            "",
            &flash(
                &locale,
                params.get("notice").map(String::as_str).unwrap_or(""),
            ),
        )
        .await;
    }
    let roles = auth
        .list_user_role_names()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(user_id, name)| (user_id == id).then_some(name))
        .collect::<Vec<_>>();
    let role_choices = role_names(auth).await;
    let permissions = auth.list_user_permissions(id).await.unwrap_or_default();
    let display_name = auth.display_name(id).await.unwrap_or_default();
    let csrf = session_csrf(auth, &headers).await;
    let history = record_history(&state, &actor, &locale, "_fexu_users", &id.to_string()).await;
    page_as(
        &state,
        &actor,
        &registry,
        "user_detail.html",
        &locale,
        theme_of(&headers),
        &target.email,
        true,
        true,
        &actor.email,
        &csrf,
        "/admin/directory/users",
        &flash(
            &locale,
            params.get("notice").map(String::as_str).unwrap_or(""),
        ),
        "",
        json!({
            "id": id,
            "email": target.email,
            "display_name": display_name,
            "is_active": target.is_active,
            "is_staff": target.is_staff,
            "is_superuser": target.is_superuser,
            "roles": roles,
            "role_choices": role_choices,
            "permissions": permissions,
            "self_record": actor.id == id,
            "can_edit": actor.is_superuser && !target.is_superuser,
            "history_href": history_href("_fexu_users", &id.to_string()),
            "history": history.events,
            "history_more": history.more
        }),
    )
    .await
}

async fn user_edit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    Path(id): Path<Uuid>,
) -> Response {
    user_form(&state, &headers, &registry, Some(id), None, "", "").await
}

async fn user_update(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    Path(id): Path<Uuid>,
    body: Bytes,
) -> Response {
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let actor = match require_directory_post(&state, &headers, &registry, &body).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    let auth = state.auth.as_ref().unwrap();
    let Some(_target) = auth.get_user(id).await.ok().flatten() else {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &actor.email,
        )
        .await;
    };
    if let Err(code) = confirmed(&body) {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::UNPROCESSABLE_ENTITY,
            code,
            true,
            &actor.email,
        )
        .await;
    }
    let values = match FormValues::parse_urlencoded(&String::from_utf8_lossy(&body)) {
        Ok(values) => values,
        Err(_) => {
            return error_page(
                &state,
                &headers,
                &registry,
                &locale,
                StatusCode::UNPROCESSABLE_ENTITY,
                "confirm_invalid",
                true,
                &actor.email,
            )
            .await
        }
    };
    if reject_unknown(
        &values,
        &["confirm", "email", "name", "active", "staff", "save"],
    )
    .is_err()
    {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::UNPROCESSABLE_ENTITY,
            "forbidden",
            true,
            &actor.email,
        )
        .await;
    }
    let email = required_text(
        &values,
        TextSpec {
            name: "email",
            required: true,
            min_length: None,
            max_length: Some(254),
            email: true,
            url: false,
        },
    );
    let active = required_bool(&values, "active");
    let staff = required_bool(&values, "staff");
    let email_invalid = email.is_err();
    let (Ok(email), Ok(active), Ok(staff)) = (email, active, staff) else {
        let message = if email_invalid {
            catalog(&locale)["invalid_credentials"]
                .as_str()
                .unwrap_or("Invalid email or password.")
                .to_owned()
        } else {
            catalog(&locale)["confirm_invalid"]
                .as_str()
                .unwrap_or("The confirmation is invalid.")
                .to_owned()
        };
        let mut draft = user_draft(&values, None);
        if email_invalid {
            draft.email_error = message.clone();
        }
        return user_form(
            &state,
            &headers,
            &registry,
            Some(id),
            Some(draft),
            &message,
            "",
        )
        .await;
    };
    if auth
        .set_display_name(id, values.get("name").unwrap_or(""))
        .await
        .is_err()
    {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::UNPROCESSABLE_ENTITY,
            "forbidden",
            true,
            &actor.email,
        )
        .await;
    }
    match crate::directory::update_user(auth, actor.id, id, &email, active, staff).await {
        Ok(_) => redirect(&if values.get("save") == Some("list") {
            "/admin/directory/users?notice=updated".to_owned()
        } else {
            format!("/admin/directory/users/{id}?notice=updated")
        }),
        Err(fexu_core::FexuError::Forbidden) => {
            error_page(
                &state,
                &headers,
                &registry,
                &locale,
                StatusCode::FORBIDDEN,
                "forbidden",
                true,
                &actor.email,
            )
            .await
        }
        Err(fexu_core::FexuError::Validation(message)) if message.contains("email") => {
            let text = catalog(&locale)["email_taken"]
                .as_str()
                .unwrap_or("That email is already registered.")
                .to_owned();
            let mut draft = user_draft(&values, None);
            draft.email_error = text.clone();
            user_form(
                &state,
                &headers,
                &registry,
                Some(id),
                Some(draft),
                &text,
                "",
            )
            .await
        }
        Err(_) => {
            error_page(
                &state,
                &headers,
                &registry,
                &locale,
                StatusCode::UNPROCESSABLE_ENTITY,
                "forbidden",
                true,
                &actor.email,
            )
            .await
        }
    }
}

async fn user_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    Path(id): Path<Uuid>,
    body: Bytes,
) -> Response {
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let actor = match require_directory_post(&state, &headers, &registry, &body).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    let auth = state.auth.as_ref().unwrap();
    let Some(_target) = auth.get_user(id).await.ok().flatten() else {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &actor.email,
        )
        .await;
    };
    if let Err(code) = confirmed(&body) {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::UNPROCESSABLE_ENTITY,
            code,
            true,
            &actor.email,
        )
        .await;
    }
    let values = FormValues::parse_urlencoded(&String::from_utf8_lossy(&body)).unwrap_or_default();
    if reject_unknown(&values, &["confirm", "password", "password_confirm"]).is_err() {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::UNPROCESSABLE_ENTITY,
            "forbidden",
            true,
            &actor.email,
        )
        .await;
    }
    let Some(password) = values.get("password") else {
        let text = catalog(&locale)["password_invalid"]
            .as_str()
            .unwrap_or("The password must contain 12 to 128 characters.")
            .to_owned();
        return user_form(
            &state,
            &headers,
            &registry,
            Some(id),
            Some(UserDraft {
                email: None,
                name: None,
                active: None,
                staff: None,
                roles: None,
                email_error: String::new(),
                password_error: text.clone(),
            }),
            &text,
            "",
        )
        .await;
    };
    let password_confirm = values.get("password_confirm");
    if password_confirm != Some(password) {
        let text = catalog(&locale)["password_mismatch"]
            .as_str()
            .unwrap_or("The passwords do not match.")
            .to_owned();
        return user_form(
            &state,
            &headers,
            &registry,
            Some(id),
            Some(UserDraft {
                email: None,
                name: None,
                active: None,
                staff: None,
                roles: None,
                email_error: String::new(),
                password_error: text.clone(),
            }),
            &text,
            "",
        )
        .await;
    }
    match crate::directory::reset_password(auth, actor.id, id, password).await {
        Ok(()) => {
            if actor.id == id {
                redirect("/admin/login")
            } else {
                redirect(&format!("/admin/directory/users/{id}?notice=updated"))
            }
        }
        Err(fexu_core::FexuError::Forbidden) => {
            error_page(
                &state,
                &headers,
                &registry,
                &locale,
                StatusCode::FORBIDDEN,
                "forbidden",
                true,
                &actor.email,
            )
            .await
        }
        Err(fexu_core::FexuError::Validation(_)) => {
            let text = catalog(&locale)["password_invalid"]
                .as_str()
                .unwrap_or("The password must contain 12 to 128 characters.")
                .to_owned();
            user_form(
                &state,
                &headers,
                &registry,
                Some(id),
                Some(UserDraft {
                    email: None,
                    name: None,
                    active: None,
                    staff: None,
                    roles: None,
                    email_error: String::new(),
                    password_error: text.clone(),
                }),
                &text,
                "",
            )
            .await
        }
        Err(_) => {
            error_page(
                &state,
                &headers,
                &registry,
                &locale,
                StatusCode::UNPROCESSABLE_ENTITY,
                "forbidden",
                true,
                &actor.email,
            )
            .await
        }
    }
}

struct UserDraft {
    email: Option<String>,
    name: Option<String>,
    active: Option<bool>,
    staff: Option<bool>,
    roles: Option<Vec<String>>,
    email_error: String,
    password_error: String,
}

fn user_draft(values: &FormValues, roles: Option<Vec<String>>) -> UserDraft {
    let flag = |name: &str| match values.get(name) {
        Some("true") | Some("1") | Some("on") => Some(true),
        Some("false") | Some("0") => Some(false),
        _ => None,
    };
    UserDraft {
        email: Some(values.get("email").unwrap_or("").to_owned()),
        name: Some(values.get("name").unwrap_or("").to_owned()),
        active: flag("active"),
        staff: flag("staff"),
        roles,
        email_error: String::new(),
        password_error: String::new(),
    }
}

async fn user_form(
    state: &AppState,
    headers: &HeaderMap,
    registry: &AdminRegistry,
    id: Option<Uuid>,
    draft: Option<UserDraft>,
    alert: &str,
    notice: &str,
) -> Response {
    let locale = locale_of(
        headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let actor = match require_directory(state, headers, registry, "/admin/directory/users").await {
        Ok(user) => user,
        Err(response) => return response,
    };
    let auth = state.auth.as_ref().unwrap();
    let target = if let Some(id) = id {
        match auth.get_user(id).await.ok().flatten() {
            Some(user) => Some(user),
            None => {
                return error_page(
                    state,
                    headers,
                    registry,
                    &locale,
                    StatusCode::NOT_FOUND,
                    "not_found",
                    true,
                    &actor.email,
                )
                .await
            }
        }
    } else {
        None
    };
    let assigned = if let Some(id) = id {
        auth.list_user_role_names()
            .await
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(user_id, name)| (user_id == id).then_some(name))
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let email = draft
        .as_ref()
        .and_then(|item| item.email.clone())
        .or_else(|| target.as_ref().map(|user| user.email.clone()))
        .unwrap_or_default();
    let name = match draft.as_ref().and_then(|item| item.name.clone()) {
        Some(name) => name,
        None => match id {
            Some(id) => auth.display_name(id).await.unwrap_or_default(),
            None => String::new(),
        },
    };
    let is_active = draft
        .as_ref()
        .and_then(|item| item.active)
        .or_else(|| target.as_ref().map(|user| user.is_active))
        .unwrap_or(true);
    let is_staff = draft
        .as_ref()
        .and_then(|item| item.staff)
        .or_else(|| target.as_ref().map(|user| user.is_staff))
        .unwrap_or(false);
    let known_roles = role_names(auth).await;
    let selected = draft
        .as_ref()
        .and_then(|item| item.roles.clone())
        .unwrap_or_default();
    let role_checks: Vec<Value> = known_roles
        .iter()
        .map(|name| {
            json!({
                "name": name,
                "checked": selected.iter().any(|role| role == name)
            })
        })
        .collect();
    let email_error = draft
        .as_ref()
        .map(|item| item.email_error.clone())
        .unwrap_or_default();
    let password_error = draft
        .as_ref()
        .map(|item| item.password_error.clone())
        .unwrap_or_default();
    let csrf = session_csrf(auth, headers).await;
    let history = if let Some(record_id) = id {
        record_history(
            state,
            &actor,
            &locale,
            "_fexu_users",
            &record_id.to_string(),
        )
        .await
    } else {
        RecordHistory::none()
    };
    let record_history_href = id
        .map(|record_id| history_href("_fexu_users", &record_id.to_string()))
        .unwrap_or_default();
    let action = match id {
        Some(id) => format!("/admin/directory/users/{id}"),
        None => "/admin/directory/users/new".to_owned(),
    };
    let mut response = page_as(
        state,
        &actor,
        registry,
        "user_form.html",
        &locale,
        theme_of(headers),
        catalog(&locale)[if id.is_some() { "edit" } else { "add_user" }]
            .as_str()
            .unwrap_or("User"),
        true,
        true,
        &actor.email,
        &csrf,
        "/admin/directory/users",
        notice,
        alert,
        json!({
            "id": id.map(|value| value.to_string()).unwrap_or_default(),
            "creating": id.is_none(),
            "action": action,
            "email": email,
            "name": name,
            "email_error": email_error,
            "password_error": password_error,
            "is_active": is_active,
            "is_staff": is_staff,
            "is_superuser": target.as_ref().is_some_and(|user| user.is_superuser),
            "self_edit": id == Some(actor.id),
            "roles": known_roles,
            "role_checks": role_checks,
            "assigned": assigned,
            "permissions": if let Some(id) = id { auth.list_user_permissions(id).await.unwrap_or_default() } else { Vec::new() },
            "history_href": record_history_href,
            "history": history.events,
            "history_more": history.more
        }),
    )
    .await;
    if !alert.is_empty() {
        *response.status_mut() = StatusCode::UNPROCESSABLE_ENTITY;
    }
    response
}

fn repeated(body: &Bytes, name: &str) -> Vec<String> {
    let mut values = Vec::new();
    for pair in String::from_utf8_lossy(body).split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if decode(key) == name {
            let value = decode(value);
            if !value.is_empty() {
                values.push(value);
            }
        }
    }
    values
}

async fn mutate_user(
    state: &AppState,
    headers: &HeaderMap,
    registry: &AdminRegistry,
    id: Uuid,
    body: &Bytes,
    active_flag: bool,
) -> Response {
    let locale = locale_of(
        headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let user = match require_directory_post(state, headers, registry, body).await {
        Ok(user) => user,
        Err(response) => return response,
    };
    let form = parse_form(body);
    let auth = state.auth.as_ref().unwrap();
    let Some(_target) = auth
        .list_users()
        .await
        .ok()
        .and_then(|users| users.into_iter().find(|candidate| candidate.id == id))
    else {
        return error_page(
            state,
            headers,
            registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    };
    if let Err(code) = confirmed(body) {
        return error_page(
            state,
            headers,
            registry,
            &locale,
            StatusCode::UNPROCESSABLE_ENTITY,
            code,
            true,
            &user.email,
        )
        .await;
    }
    let result = if active_flag {
        crate::directory::set_active(
            auth,
            user.id,
            id,
            form_flag(&form, "active").unwrap_or(false),
        )
        .await
    } else {
        crate::directory::set_staff(
            auth,
            user.id,
            id,
            form_flag(&form, "staff").unwrap_or(false),
        )
        .await
    };
    if result.is_err() {
        return error_page(
            state,
            headers,
            registry,
            &locale,
            StatusCode::UNPROCESSABLE_ENTITY,
            "forbidden",
            true,
            &user.email,
        )
        .await;
    }
    redirect("/admin/directory/users?notice=updated")
}

#[allow(clippy::result_large_err)]
pub(crate) async fn require_get(
    state: &AppState,
    headers: &HeaderMap,
    registry: &AdminRegistry,
    next: &str,
) -> Result<AuthUser, Response> {
    let Some(auth) = state.auth.as_ref() else {
        return Err(plain(
            StatusCode::FORBIDDEN,
            "authentication is not configured",
        ));
    };
    match auth.authenticate(headers, &Method::GET).await {
        Ok(user) if user.is_staff => Ok(user),
        Ok(user) => {
            let locale = locale_of(
                headers,
                &registry.branding().default_locale,
                &registry.branding().fallback_locale,
            );
            Err(error_page(
                state,
                headers,
                registry,
                &locale,
                StatusCode::FORBIDDEN,
                "staff_required",
                true,
                &user.email,
            )
            .await)
        }
        Err(AuthError::Unauthenticated) => Err(redirect(&format!(
            "/admin/login?next={}",
            url_encode(safe_next(next).unwrap_or("/admin"))
        ))),
        Err(_) => Err(plain(StatusCode::FORBIDDEN, "authentication failed")),
    }
}

pub(crate) async fn require_post(
    state: &AppState,
    headers: &HeaderMap,
    body: &Bytes,
) -> Result<AuthUser, ()> {
    let Some(auth) = state.auth.as_ref() else {
        return Err(());
    };
    let token = csrf_token_from(headers, body);
    let user = auth
        .authenticate_csrf(headers, &Method::POST, &token)
        .await
        .map_err(|_| ())?;
    user.is_staff.then_some(user).ok_or(())
}

#[allow(clippy::result_large_err)]
async fn require_directory(
    state: &AppState,
    headers: &HeaderMap,
    registry: &AdminRegistry,
    next: &str,
) -> Result<AuthUser, Response> {
    let user = require_get(state, headers, registry, next).await?;
    if user.is_superuser {
        Ok(user)
    } else {
        let locale = locale_of(
            headers,
            &registry.branding().default_locale,
            &registry.branding().fallback_locale,
        );
        Err(error_page(
            state,
            headers,
            registry,
            &locale,
            StatusCode::FORBIDDEN,
            "forbidden",
            true,
            &user.email,
        )
        .await)
    }
}

#[allow(clippy::result_large_err)]
async fn require_directory_post(
    state: &AppState,
    headers: &HeaderMap,
    registry: &AdminRegistry,
    body: &Bytes,
) -> Result<AuthUser, Response> {
    let locale = locale_of(
        headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let Some(auth) = state.auth.as_ref() else {
        return Err(plain(
            StatusCode::FORBIDDEN,
            "authentication is not configured",
        ));
    };
    let form = parse_form(body);
    match auth
        .authenticate_csrf(
            headers,
            &Method::POST,
            form.get("csrf_token").map(String::as_str).unwrap_or(""),
        )
        .await
    {
        Ok(user) if user.is_superuser => Ok(user),
        Ok(user) => Err(error_page(
            state,
            headers,
            registry,
            &locale,
            StatusCode::FORBIDDEN,
            "forbidden",
            true,
            &user.email,
        )
        .await),
        Err(_) => Err(error_page(
            state,
            headers,
            registry,
            &locale,
            StatusCode::FORBIDDEN,
            "csrf",
            false,
            "",
        )
        .await),
    }
}

#[allow(clippy::result_large_err)]
async fn require_roles(
    state: &AppState,
    headers: &HeaderMap,
    registry: &AdminRegistry,
    next: &str,
) -> Result<AuthUser, Response> {
    let user = require_get(state, headers, registry, next).await?;
    let allowed = match state.auth.as_ref() {
        Some(auth) => auth.holds(&user, "roles.update").await.unwrap_or(false),
        None => false,
    };
    if allowed {
        Ok(user)
    } else {
        let locale = locale_of(
            headers,
            &registry.branding().default_locale,
            &registry.branding().fallback_locale,
        );
        Err(error_page(
            state,
            headers,
            registry,
            &locale,
            StatusCode::FORBIDDEN,
            "forbidden",
            true,
            &user.email,
        )
        .await)
    }
}

#[allow(clippy::result_large_err)]
async fn require_roles_post(
    state: &AppState,
    headers: &HeaderMap,
    registry: &AdminRegistry,
    body: &Bytes,
) -> Result<AuthUser, Response> {
    let locale = locale_of(
        headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let Some(auth) = state.auth.as_ref() else {
        return Err(plain(
            StatusCode::FORBIDDEN,
            "authentication is not configured",
        ));
    };
    let form = parse_form(body);
    let user = match auth
        .authenticate_csrf(
            headers,
            &Method::POST,
            form.get("csrf_token").map(String::as_str).unwrap_or(""),
        )
        .await
    {
        Ok(user) => user,
        Err(_) => {
            return Err(error_page(
                state,
                headers,
                registry,
                &locale,
                StatusCode::FORBIDDEN,
                "csrf",
                false,
                "",
            )
            .await)
        }
    };
    if auth.holds(&user, "roles.update").await.unwrap_or(false) {
        Ok(user)
    } else {
        Err(error_page(
            state,
            headers,
            registry,
            &locale,
            StatusCode::FORBIDDEN,
            "forbidden",
            true,
            &user.email,
        )
        .await)
    }
}

#[allow(clippy::too_many_arguments)]
async fn model_draft(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    Path(model_key): Path<String>,
    body: Bytes,
) -> Response {
    keep_draft(&state, &headers, &registry, &model_key, None, &body).await
}

async fn model_record_draft(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    Path((model_key, id)): Path<(String, Uuid)>,
    body: Bytes,
) -> Response {
    keep_draft(&state, &headers, &registry, &model_key, Some(id), &body).await
}

async fn keep_draft(
    state: &AppState,
    headers: &HeaderMap,
    registry: &AdminRegistry,
    model_key: &str,
    id: Option<Uuid>,
    body: &Bytes,
) -> Response {
    let locale = locale_of(
        headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let user = match require_post(state, headers, body).await {
        Ok(user) => user,
        Err(_) => {
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
            .await
        }
    };
    let Some(model) = registry.find(model_key) else {
        return error_page(
            state,
            headers,
            registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    };
    let action_name = if id.is_some() { "update" } else { "create" };
    if gate(state, registry, &user, model.as_ref(), action_name)
        .await
        .is_err()
    {
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
    let form = parse_form(body);
    let Some(field) = form.get("pick_field").map(String::as_str) else {
        return error_page(
            state,
            headers,
            registry,
            &locale,
            StatusCode::UNPROCESSABLE_ENTITY,
            "forbidden",
            true,
            &user.email,
        )
        .await;
    };
    let allowed = registry.field_slice(model.meta()).iter().any(|item| {
        item.name == field
            && !item.hidden
            && !item.readonly
            && !item.sensitive
            && !sensitive(item.name)
    }) && model
        .columns()
        .iter()
        .any(|column| column.name == field && column.foreign_table.is_some());
    if !allowed {
        return error_page(
            state,
            headers,
            registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    }
    let values = draft_values(model.meta(), model.columns(), &form);
    let record = id.map(|value| value.to_string());
    let draft = drafts::insert(
        &state.db,
        user.session_id,
        user.id,
        model_key,
        record.as_deref(),
        "form",
        &values,
    )
    .await;
    let Ok(draft) = draft else {
        return error_page(
            state,
            headers,
            registry,
            &locale,
            StatusCode::UNPROCESSABLE_ENTITY,
            "forbidden",
            true,
            &user.email,
        )
        .await;
    };
    let target = match id {
        Some(id) => format!("/admin/models/{model_key}/{id}/pick/{field}?draft={draft}"),
        None => format!("/admin/models/{model_key}/pick/{field}?draft={draft}"),
    };
    redirect(&target)
}

async fn inline_new(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    Path((model_key, id, inline)): Path<(String, Uuid, String)>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let user = match require_get(
        &state,
        &headers,
        &registry,
        &format!("/admin/models/{model_key}/{id}"),
    )
    .await
    {
        Ok(user) => user,
        Err(response) => return response,
    };
    let Some(parent) = registry.find(&model_key) else {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale_of(
                &headers,
                &registry.branding().default_locale,
                &registry.branding().fallback_locale,
            ),
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    };
    let Some(child) = child_inline(&registry, parent.as_ref(), &inline) else {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale_of(
                &headers,
                &registry.branding().default_locale,
                &registry.branding().fallback_locale,
            ),
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    };
    if read_visible(&state, &user, parent.as_ref(), id, "read")
        .await
        .is_none()
        || gate(&state, &registry, &user, child.as_ref(), "create")
            .await
            .is_err()
    {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale_of(
                &headers,
                &registry.branding().default_locale,
                &registry.branding().fallback_locale,
            ),
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    }
    let spec = parent
        .meta()
        .inlines
        .iter()
        .find(|item| item.name == inline)
        .expect("inline");
    let mut values = Map::new();
    values.insert(spec.column.to_owned(), json!(id.to_string()));
    values.insert("_lock".to_owned(), json!(spec.column));
    values.insert(
        "_action".to_owned(),
        json!(format!(
            "/admin/models/{model_key}/{id}/inline/{inline}/new"
        )),
    );
    values.insert(
        "_cancel".to_owned(),
        json!(format!("/admin/models/{model_key}/{id}")),
    );
    if query.contains_key("draft") {
        values.insert("_draft".to_owned(), json!(query.get("draft")));
        values.insert("_use_draft".to_owned(), json!(true));
    }
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    form_page(
        &state,
        &headers,
        &registry,
        child.meta().key,
        None,
        values,
        FormFeedback {
            notice: &flash(&locale, ""),
            alert: "",
        },
    )
    .await
}

async fn inline_create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    Path((model_key, id, inline)): Path<(String, Uuid, String)>,
    body: Bytes,
) -> Response {
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let user = match require_post(&state, &headers, &body).await {
        Ok(user) => user,
        Err(_) => {
            return error_page(
                &state,
                &headers,
                &registry,
                &locale,
                StatusCode::FORBIDDEN,
                "csrf",
                true,
                "",
            )
            .await
        }
    };
    let Some(parent) = registry.find(&model_key) else {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    };
    let Some(child) = child_inline(&registry, parent.as_ref(), &inline) else {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    };
    let Some(spec) = parent
        .meta()
        .inlines
        .iter()
        .find(|item| item.name == inline)
    else {
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    };
    if read_visible(&state, &user, parent.as_ref(), id, "read")
        .await
        .is_none()
        || gate(&state, &registry, &user, child.as_ref(), "create")
            .await
            .is_err()
    {
        let _ = audit::insert(
            &state.db,
            &Event {
                actor_id: user.id,
                model_key: child.meta().key.to_owned(),
                record_id: String::new(),
                action: "create".to_owned(),
                outcome: "denied".to_owned(),
                summary: String::new(),
                operation_id: None,
            },
        )
        .await;
        return error_page(
            &state,
            &headers,
            &registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    }
    let form = parse_form(&body);
    let (mut object, mut raw) = payload(child.meta(), child.columns(), &form);
    object.insert(spec.column.to_owned(), json!(id.to_string()));
    raw.insert(spec.column.to_owned(), json!(id.to_string()));
    raw.insert("_lock".to_owned(), json!(spec.column));
    raw.insert(
        "_action".to_owned(),
        json!(format!(
            "/admin/models/{model_key}/{id}/inline/{inline}/new"
        )),
    );
    raw.insert(
        "_cancel".to_owned(),
        json!(format!("/admin/models/{model_key}/{id}")),
    );
    if let Err(message) = reject_locked(registry.field_slice(child.meta()), &object, true) {
        let text = validation_text(&locale, &message);
        raw.insert(
            "__error".into(),
            json!({ "field": message.split_whitespace().next().unwrap_or(""), "text": text }),
        );
        return form_page(
            &state,
            &headers,
            &registry,
            child.meta().key,
            None,
            raw,
            FormFeedback {
                notice: "",
                alert: &text,
            },
        )
        .await;
    }
    if let Err(field) = ensure_relations(&state, &registry, &user, child.columns(), &object).await {
        let text = catalog(&locale)
            .get("invalid_relation")
            .and_then(Value::as_str)
            .unwrap_or("Choose a visible record.")
            .to_owned();
        raw.insert("__error".into(), json!({ "field": field, "text": text }));
        return form_page(
            &state,
            &headers,
            &registry,
            child.meta().key,
            None,
            raw,
            FormFeedback {
                notice: "",
                alert: &text,
            },
        )
        .await;
    }
    match child.create(&state.db, Value::Object(object), &user).await {
        Ok(_) => redirect(&format!("/admin/models/{model_key}/{id}?notice=created")),
        Err(FexuError::Validation(message)) => {
            let text = validation_text(&locale, &message);
            let field = message.split_whitespace().next().unwrap_or("");
            raw.insert("__error".into(), json!({ "field": field, "text": text }));
            form_page(
                &state,
                &headers,
                &registry,
                child.meta().key,
                None,
                raw,
                FormFeedback {
                    notice: "",
                    alert: &text,
                },
            )
            .await
        }
        Err(FexuError::Denied(_)) => {
            error_page(
                &state,
                &headers,
                &registry,
                &locale,
                StatusCode::FORBIDDEN,
                "forbidden",
                true,
                &user.email,
            )
            .await
        }
        Err(_) => {
            error_page(
                &state,
                &headers,
                &registry,
                &locale,
                StatusCode::UNPROCESSABLE_ENTITY,
                "forbidden",
                true,
                &user.email,
            )
            .await
        }
    }
}

async fn model_bulk(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    Path(model_key): Path<String>,
    body: Bytes,
) -> Response {
    stage_bulk(&state, &headers, &registry, &model_key, &body).await
}

async fn bulk_confirm(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    Path((model_key, draft)): Path<(String, Uuid)>,
) -> Response {
    show_bulk(&state, &headers, &registry, &model_key, draft).await
}

async fn bulk_apply(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    Path((model_key, draft)): Path<(String, Uuid)>,
    body: Bytes,
) -> Response {
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let user = match require_post(&state, &headers, &body).await {
        Ok(user) => user,
        Err(_) => {
            return error_page(
                &state,
                &headers,
                &registry,
                &locale,
                StatusCode::FORBIDDEN,
                "csrf",
                true,
                "",
            )
            .await
        }
    };
    if confirmed(&body).is_err() {
        return show_bulk(&state, &headers, &registry, &model_key, draft).await;
    }
    apply_bulk(
        &state, &headers, &registry, &user, &locale, &model_key, draft,
    )
    .await
}

async fn audit_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(registry): Extension<AdminRegistry>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let locale = locale_of(
        &headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let user = match require_get(&state, &headers, &registry, "/admin/audit").await {
        Ok(user) => user,
        Err(response) => return response,
    };
    let page_number = params
        .get("page")
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(1)
        .max(1);
    let actor = (!user.is_superuser).then_some(user.id);
    let model_filter = params.get("model").and_then(|value| audit_key(value));
    let record_filter = params.get("record").and_then(|value| audit_key(value));
    let listed = if model_filter.is_none() && record_filter.is_none() {
        audit::page(
            &state.db,
            actor,
            u64::from(page_number - 1) * audit::PAGE_SIZE,
        )
        .await
    } else {
        audit::matching(
            &state.db,
            actor,
            model_filter.as_deref(),
            record_filter.as_deref(),
            u64::from(page_number - 1) * audit::PAGE_SIZE,
        )
        .await
    };
    let (events, has_more) = match listed {
        Ok(value) => value,
        Err(_) => {
            return error_page(
                &state,
                &headers,
                &registry,
                &locale,
                StatusCode::INTERNAL_SERVER_ERROR,
                "forbidden",
                true,
                &user.email,
            )
            .await
        }
    };
    let rows: Vec<Value> = events
        .into_iter()
        .map(|event| {
            let actor_label = if event.actor_id == user.id {
                user.email.clone()
            } else {
                event.actor_id.to_string()
            };
            let formatted = audit::format_unix_utc(event.created_at);
            let record_href = (registry.find(&event.model_key).is_some()
                && Uuid::parse_str(&event.record_id).is_ok())
            .then(|| format!("/admin/models/{}/{}", event.model_key, event.record_id));
            json!({
                "when": formatted.as_ref().map(|(label, _)| label.clone()).unwrap_or_default(),
                "when_iso": formatted.as_ref().map(|(_, iso)| iso.clone()).unwrap_or_default(),
                "actor": actor_label,
                "action": event.action,
                "record": event.record_id,
                "record_short": event.record_id.chars().take(8).collect::<String>(),
                "record_href": record_href,
                "ok": event.outcome == "ok",
                "operation": event
                    .operation_id
                    .map(|id| id.to_string().chars().take(8).collect::<String>())
                    .unwrap_or_default(),
                "summary": event.summary
            })
        })
        .collect();
    let csrf = session_csrf(state.auth.as_ref().unwrap(), &headers).await;
    page_as(
        &state,
        &user,
        &registry,
        "audit.html",
        &locale,
        theme_of(&headers),
        catalog(&locale)
            .get("history")
            .and_then(Value::as_str)
            .unwrap_or("History"),
        true,
        user.is_superuser,
        &user.email,
        &csrf,
        "/admin/audit",
        "",
        "",
        json!({
            "rows": rows,
            "filtered": model_filter.is_some() || record_filter.is_some(),
            "filter_label": match (&model_filter, &record_filter) {
                (Some(model), Some(record)) => format!("{model} · {record}"),
                (Some(model), None) => model.clone(),
                (None, Some(record)) => record.clone(),
                (None, None) => String::new(),
            },
            "previous": (page_number > 1).then(|| audit_page_href(page_number - 1, model_filter.as_deref(), record_filter.as_deref())),
            "next_page": has_more.then(|| audit_page_href(page_number + 1, model_filter.as_deref(), record_filter.as_deref()))
        }),
    )
    .await
}

fn child_inline<'a>(
    registry: &'a AdminRegistry,
    parent: &dyn ManagedModel,
    name: &str,
) -> Option<&'a std::sync::Arc<dyn ManagedModel>> {
    let spec = parent
        .meta()
        .inlines
        .iter()
        .find(|item| item.name == name)?;
    resolve_inline(registry, parent, spec)
}

fn resolve_inline<'a>(
    registry: &'a AdminRegistry,
    parent: &dyn ManagedModel,
    spec: &fexu_core::AdminInline,
) -> Option<&'a std::sync::Arc<dyn ManagedModel>> {
    let child = registry.find(spec.child_table)?;
    child.columns().iter().find(|column| {
        column.name == spec.column && crate::registry::references_parent(column, parent)
    })?;
    Some(child)
}

fn draft_values(
    meta: &fexu_core::AdminModelMeta,
    columns: &[ColumnSpec],
    form: &HashMap<String, String>,
) -> Map<String, Value> {
    let mut values = Map::new();
    for field in meta.fields {
        if field.hidden
            || field.readonly
            || field.sensitive
            || sensitive(field.name)
            || field.name == "id"
        {
            continue;
        }
        let Some(value) = form.get(field.name) else {
            continue;
        };
        if value.chars().count() > 2_000 {
            continue;
        }
        if columns.iter().any(|column| column.name == field.name) {
            values.insert(field.name.to_owned(), json!(value));
        }
    }
    for (key, value) in form {
        if (key.starts_with("inline_") || key.starts_with("extra_") || key == "basis")
            && value.chars().count() <= 2_000
            && !key.contains("password")
        {
            values.insert(key.clone(), json!(value));
        }
    }
    values
}

async fn take_draft(
    state: &AppState,
    user: &AuthUser,
    model_key: &str,
    id: Option<Uuid>,
    values: &mut Map<String, Value>,
) -> String {
    let picks = match values.remove("_picks") {
        Some(Value::Object(map)) => map,
        _ => Map::new(),
    };
    let use_draft = values
        .remove("_use_draft")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let token = values
        .get("_draft")
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok());
    let mut shown = String::new();
    if use_draft {
        if let Some(token) = token {
            if let Ok(Some(draft)) = drafts::load(
                &state.db,
                token,
                user.session_id,
                user.id,
                model_key,
                "form",
            )
            .await
            {
                let record = draft.record_id.unwrap_or_default();
                let matches = match id {
                    Some(id) => record == id.to_string(),
                    None => record.is_empty(),
                };
                if matches {
                    for (key, value) in draft.values {
                        values.entry(key).or_insert(value);
                    }
                    shown = token.to_string();
                }
            }
        }
    } else if let Some(token) = token {
        shown = token.to_string();
    }
    for (key, value) in picks {
        values.insert(key, value);
    }
    if shown.is_empty() {
        values.remove("_draft");
    } else {
        values.insert("_draft".to_owned(), json!(shown));
    }
    shown
}

fn choice_href(back: &str, field: &str, id: &str, draft: Option<&str>) -> String {
    match draft.filter(|value| Uuid::parse_str(value).is_ok()) {
        Some(draft) => format!("{back}?draft={}&{field}={id}", url_encode(draft)),
        None => format!("{back}?{field}={id}"),
    }
}

#[allow(clippy::too_many_arguments)]
async fn inline_sections(
    state: &AppState,
    registry: &AdminRegistry,
    user: &AuthUser,
    parent: &dyn ManagedModel,
    parent_id: Uuid,
    query: &HashMap<String, String>,
    editing: bool,
    locale: &str,
) -> Vec<Value> {
    let mut sections = Vec::new();
    let mut specs: Vec<fexu_core::AdminInline> = parent.meta().inlines.to_vec();
    for table in registry.related_tables(parent.meta().key) {
        if specs.iter().any(|spec| spec.name == table.name) {
            continue;
        }
        specs.push(fexu_core::AdminInline {
            name: table.name,
            child_table: table.child_key,
            column: table.column,
            label: table.label,
            fields: &[],
            readonly_fields: &[],
            min_count: 0,
            max_count: 0,
            extra: 0,
            can_add: true,
            can_change: true,
            can_delete: false,
            ordering: None,
            form: false,
        });
    }
    for spec in &specs {
        if editing && spec.form {
            continue;
        }
        let Some(child) = resolve_inline(registry, parent, spec) else {
            continue;
        };
        if list_blocked(child.as_ref())
            || permit(state.auth.as_ref().unwrap(), user, child.meta(), "read")
                .await
                .is_err()
        {
            continue;
        }
        let page_number = query
            .get(&format!("inline.{}", spec.name))
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(1)
            .clamp(1, 10_000);
        let limit = u64::from(registry.per_page_of(child.meta()).clamp(1, 25));
        let scope = access_scope(child.as_ref(), user, false);
        let filters = [(spec.column, DbValue::from(parent_id))];
        let Ok((rows, has_more, _)) = child
            .page(
                &state.db,
                &[],
                None,
                "id",
                false,
                &filters,
                &[],
                u64::from(page_number - 1) * limit,
                limit,
                &scope,
            )
            .await
        else {
            continue;
        };
        let owned = parent
            .meta()
            .inlines
            .iter()
            .any(|item| item.name == spec.name);
        if !owned {
            let shown: Vec<&str> = child
                .meta()
                .fields
                .iter()
                .filter(|field| {
                    field.list_display
                        && !field.hidden
                        && !field.sensitive
                        && field.name != "id"
                        && field.name != spec.column
                })
                .map(|field| field.name)
                .collect();
            let shown: Vec<&str> = if shown.is_empty() { vec!["id"] } else { shown };
            let foreign: Vec<ColumnSpec> = child
                .columns()
                .iter()
                .copied()
                .filter(|column| shown.contains(&column.name) && column.foreign_table.is_some())
                .collect();
            let relations = if foreign.is_empty() {
                HashMap::new()
            } else {
                match relation_names(registry, state, user, &foreign, &rows).await {
                    Ok(relations) => relations,
                    Err(_) => continue,
                }
            };
            let headings: Vec<&str> = shown
                .iter()
                .map(|name| {
                    child
                        .meta()
                        .fields
                        .iter()
                        .find(|field| field.name == *name)
                        .map(|field| field.label)
                        .unwrap_or(name)
                })
                .collect();
            let child_key = child.meta().key;
            let row_ids: Vec<Uuid> = rows
                .iter()
                .filter_map(|row| {
                    row.get("id")
                        .and_then(Value::as_str)
                        .and_then(|value| Uuid::parse_str(value).ok())
                })
                .collect();
            let mut images = HashMap::new();
            for name in &shown {
                let image = registry.widgets().iter().any(|widget| {
                    widget.model_key == child_key
                        && widget.field == *name
                        && matches!(widget.kind, crate::widgets::WidgetKind::Image)
                });
                if image {
                    if let Ok(found) =
                        crate::files::stored_ids(&state.db, child_key, name, &row_ids).await
                    {
                        images.insert(*name, found);
                    }
                }
            }
            let rendered = rows
                .iter()
                .filter_map(|row| {
                    let id = row.get("id")?.as_str()?;
                    let cells: Vec<Value> = shown
                        .iter()
                        .map(|name| {
                            let mut lead = false;
                            let cell = present_cell(
                                locale,
                                registry.branding().currency.as_str(),
                                child.columns(),
                                name,
                                row.get(*name).unwrap_or(&Value::Null),
                                &relations,
                                user.is_superuser,
                                &mut lead,
                            );
                            let image = images.get(name).and_then(|found| {
                                Uuid::parse_str(id)
                                    .ok()
                                    .filter(|value| found.contains(value))
                                    .map(|value| {
                                        format!("/admin/models/{child_key}/{value}/file/{name}")
                                    })
                            });
                            json!({
                                "text": cell["text"].as_str().unwrap_or(""),
                                "href": cell["href"].as_str().unwrap_or(""),
                                "image": image.unwrap_or_default()
                            })
                        })
                        .collect();
                    Some(json!({
                        "cells": cells,
                        "href": format!("/admin/models/{}/{}", child.meta().key, id)
                    }))
                })
                .collect::<Vec<_>>();
            let can_create = permit(state.auth.as_ref().unwrap(), user, child.meta(), "create")
                .await
                .is_ok();
            sections.push(json!({
                "label": if spec.label.is_empty() { child.meta().label } else { spec.label },
                "wide": true,
                "pictures": !images.is_empty(),
                "headings": headings,
                "rows": rendered,
                "add": can_create.then(|| format!("/admin/models/{}/new", child.meta().key)),
                "next": has_more.then(|| format!("/admin/models/{}/{parent_id}?inline.{}={}", parent.meta().key, spec.name, page_number + 1))
            }));
            continue;
        }
        let label_field = child
            .meta()
            .fields
            .iter()
            .find(|field| {
                field.list_display
                    && !field.hidden
                    && !field.sensitive
                    && field.name != spec.column
                    && field.name != "id"
            })
            .map(|field| field.name)
            .unwrap_or("id");
        let foreign_label = child
            .columns()
            .iter()
            .any(|column| column.name == label_field && column.foreign_table.is_some());
        let relations = if foreign_label {
            let label_columns: Vec<ColumnSpec> = child
                .columns()
                .iter()
                .copied()
                .filter(|column| column.name == label_field)
                .collect();
            match relation_names(registry, state, user, &label_columns, &rows).await {
                Ok(relations) => relations,
                Err(_) => continue,
            }
        } else {
            HashMap::new()
        };
        let rendered = rows
            .iter()
            .filter_map(|row| {
                let id = row.get("id")?.as_str()?;
                let (text, relation_href) = if foreign_label {
                    let mut lead = false;
                    let cell = present_cell(
                        locale,
                        registry.branding().currency.as_str(),
                        child.columns(),
                        label_field,
                        row.get(label_field).unwrap_or(&Value::Null),
                        &relations,
                        user.is_superuser,
                        &mut lead,
                    );
                    (
                        cell["text"].as_str().unwrap_or(id).to_owned(),
                        cell["href"].as_str().unwrap_or("").to_owned(),
                    )
                } else {
                    let text = row
                        .get(label_field)
                        .map(|value| match value {
                            Value::String(text) => text.clone(),
                            Value::Number(number) => number.to_string(),
                            Value::Bool(flag) => flag.to_string(),
                            _ => id.to_owned(),
                        })
                        .unwrap_or_else(|| id.to_owned());
                    (text, String::new())
                };
                Some(json!({
                    "text": text,
                    "href": format!("/admin/models/{}/{}", child.meta().key, id),
                    "relation_href": relation_href
                }))
            })
            .collect::<Vec<_>>();
        let can_create = permit(state.auth.as_ref().unwrap(), user, child.meta(), "create")
            .await
            .is_ok();
        sections.push(json!({
            "label": child.meta().label,
            "rows": rendered,
            "add": (can_create && !spec.form).then(|| format!("/admin/models/{}/{parent_id}/inline/{}/new", parent.meta().key, spec.name)),
            "next": has_more.then(|| format!("/admin/models/{}/{parent_id}?inline.{}={}", parent.meta().key, spec.name, page_number + 1))
        }));
    }
    sections
}

async fn stage_bulk(
    state: &AppState,
    headers: &HeaderMap,
    registry: &AdminRegistry,
    model_key: &str,
    body: &Bytes,
) -> Response {
    let locale = locale_of(
        headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let user = match require_post(state, headers, body).await {
        Ok(user) => user,
        Err(_) => {
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
            .await
        }
    };
    let Some(model) = registry.find(model_key) else {
        return error_page(
            state,
            headers,
            registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    };
    let form = parse_form(body);
    let Some(name) = form.get("action").map(String::as_str) else {
        return error_page(
            state,
            headers,
            registry,
            &locale,
            StatusCode::UNPROCESSABLE_ENTITY,
            "forbidden",
            true,
            &user.email,
        )
        .await;
    };
    let Some(action) = registry
        .actions_for(model.meta(), model.columns())
        .into_iter()
        .find(|action| action.name == name)
    else {
        return error_page(
            state,
            headers,
            registry,
            &locale,
            StatusCode::UNPROCESSABLE_ENTITY,
            "forbidden",
            true,
            &user.email,
        )
        .await;
    };
    if permit(
        state.auth.as_ref().unwrap(),
        &user,
        model.meta(),
        action.permission,
    )
    .await
    .is_err()
    {
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
    let ids = if form.get("scope").map(String::as_str) == Some("page") {
        let (sort, descending) = match resolved_sort(
            registry.field_slice(model.meta()),
            registry.ordering_of(model.meta()),
            &form,
        ) {
            Ok(value) => value,
            Err(()) => {
                return error_page(
                    state,
                    headers,
                    registry,
                    &locale,
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "forbidden",
                    true,
                    &user.email,
                )
                .await
            }
        };
        let filters = match filters_from(
            &String::from_utf8_lossy(body),
            model.columns(),
            registry,
            model.meta(),
            registry.filters(),
        ) {
            Ok(filters) => filters,
            Err(_) => {
                return error_page(
                    state,
                    headers,
                    registry,
                    &locale,
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "forbidden",
                    true,
                    &user.email,
                )
                .await
            }
        };
        let page_number = form
            .get("page")
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(1)
            .max(1);
        let page_size = u64::from(registry.per_page_of(model.meta()).clamp(1, 100));
        let search: Vec<&str> = registry
            .field_slice(model.meta())
            .iter()
            .filter(|field| field.searchable && !field.hidden && !sensitive(field.name))
            .map(|field| field.name)
            .collect();
        let query = form.get("q").map(String::as_str).unwrap_or("");
        let listed = model
            .page(
                &state.db,
                &search,
                (!query.is_empty()).then_some(query),
                &sort,
                descending,
                &filters,
                &[],
                u64::from(page_number - 1) * page_size,
                page_size,
                &access_scope(model.as_ref(), &user, true),
            )
            .await;
        match listed {
            Ok((rows, _, _)) => rows
                .iter()
                .filter_map(|row| {
                    row.get("id")
                        .and_then(Value::as_str)
                        .and_then(|value| Uuid::parse_str(value).ok())
                })
                .collect::<Vec<_>>(),
            Err(_) => {
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
                .await
            }
        }
    } else {
        let mut ids = Vec::new();
        for value in repeated(body, "ids") {
            let Ok(id) = Uuid::parse_str(&value) else {
                return error_page(
                    state,
                    headers,
                    registry,
                    &locale,
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "forbidden",
                    true,
                    &user.email,
                )
                .await;
            };
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        ids
    };
    if ids.is_empty() {
        return error_page(
            state,
            headers,
            registry,
            &locale,
            StatusCode::UNPROCESSABLE_ENTITY,
            "bulk_empty",
            true,
            &user.email,
        )
        .await;
    }
    if ids.len() > action.max_ids {
        return error_page(
            state,
            headers,
            registry,
            &locale,
            StatusCode::UNPROCESSABLE_ENTITY,
            "forbidden",
            true,
            &user.email,
        )
        .await;
    }
    let verb = if action.permission == "delete" {
        "delete"
    } else {
        "update"
    };
    let scope = access_scope(model.as_ref(), &user, true);
    for id in &ids {
        match model.read(&state.db, *id, &scope).await {
            Ok(Some(row)) if model.object_allows(&user, verb, &row) => {}
            Ok(_) => {
                return error_page(
                    state,
                    headers,
                    registry,
                    &locale,
                    StatusCode::NOT_FOUND,
                    "bulk_denied",
                    true,
                    &user.email,
                )
                .await
            }
            Err(_) => {
                return error_page(
                    state,
                    headers,
                    registry,
                    &locale,
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "bulk_failed",
                    true,
                    &user.email,
                )
                .await
            }
        }
    }
    let mut payload = Map::new();
    payload.insert("action".to_owned(), json!(action.name));
    payload.insert(
        "ids".to_owned(),
        json!(ids.iter().map(ToString::to_string).collect::<Vec<_>>()),
    );
    let draft = drafts::insert(
        &state.db,
        user.session_id,
        user.id,
        model_key,
        None,
        "bulk",
        &payload,
    )
    .await;
    let Ok(draft) = draft else {
        return error_page(
            state,
            headers,
            registry,
            &locale,
            StatusCode::UNPROCESSABLE_ENTITY,
            "forbidden",
            true,
            &user.email,
        )
        .await;
    };
    redirect(&format!("/admin/models/{model_key}/bulk/{draft}"))
}

async fn show_bulk(
    state: &AppState,
    headers: &HeaderMap,
    registry: &AdminRegistry,
    model_key: &str,
    draft: Uuid,
) -> Response {
    let locale = locale_of(
        headers,
        &registry.branding().default_locale,
        &registry.branding().fallback_locale,
    );
    let user = match require_get(
        state,
        headers,
        registry,
        &format!("/admin/models/{model_key}"),
    )
    .await
    {
        Ok(user) => user,
        Err(response) => return response,
    };
    let Some(model) = registry.find(model_key) else {
        return error_page(
            state,
            headers,
            registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    };
    let Some(stored) = drafts::load(
        &state.db,
        draft,
        user.session_id,
        user.id,
        model_key,
        "bulk",
    )
    .await
    .ok()
    .flatten() else {
        return error_page(
            state,
            headers,
            registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    };
    let Some(name) = stored.values.get("action").and_then(Value::as_str) else {
        return error_page(
            state,
            headers,
            registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    };
    let Some(action) = registry
        .actions_for(model.meta(), model.columns())
        .into_iter()
        .find(|item| item.name == name)
    else {
        return error_page(
            state,
            headers,
            registry,
            &locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    };
    let count = stored
        .values
        .get("ids")
        .and_then(Value::as_array)
        .map(|ids| ids.len())
        .unwrap_or(0);
    let csrf = session_csrf(state.auth.as_ref().unwrap(), headers).await;
    page_as(
        state,
        &user,
        registry,
        "bulk.html",
        &locale,
        theme_of(headers),
        catalog(&locale)
            .get("bulk_confirm")
            .and_then(Value::as_str)
            .unwrap_or("Confirm"),
        true,
        user.is_superuser,
        &user.email,
        &csrf,
        &format!("/admin/models/{model_key}"),
        "",
        "",
        json!({
            "model_label": model.meta().label,
            "list_path": format!("/admin/models/{model_key}"),
            "action": format!("/admin/models/{model_key}/bulk/{draft}"),
            "action_label": if locale == "de" { action.label_de } else { action.label_en },
            "count": count
        }),
    )
    .await
}

async fn apply_bulk(
    state: &AppState,
    headers: &HeaderMap,
    registry: &AdminRegistry,
    user: &AuthUser,
    locale: &str,
    model_key: &str,
    draft: Uuid,
) -> Response {
    let Some(model) = registry.find(model_key) else {
        return error_page(
            state,
            headers,
            registry,
            locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    };
    let Some(stored) = drafts::load(
        &state.db,
        draft,
        user.session_id,
        user.id,
        model_key,
        "bulk",
    )
    .await
    .ok()
    .flatten() else {
        return error_page(
            state,
            headers,
            registry,
            locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    };
    let Some(name) = stored.values.get("action").and_then(Value::as_str) else {
        return error_page(
            state,
            headers,
            registry,
            locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    };
    let Some(action) = registry
        .actions_for(model.meta(), model.columns())
        .into_iter()
        .find(|item| item.name == name)
    else {
        return error_page(
            state,
            headers,
            registry,
            locale,
            StatusCode::UNPROCESSABLE_ENTITY,
            "forbidden",
            true,
            &user.email,
        )
        .await;
    };
    if permit(
        state.auth.as_ref().unwrap(),
        user,
        model.meta(),
        action.permission,
    )
    .await
    .is_err()
    {
        let _ = audit::insert(
            &state.db,
            &Event {
                actor_id: user.id,
                model_key: model.meta().key.to_owned(),
                record_id: String::new(),
                action: if action.permission == "delete" {
                    "bulk_delete"
                } else {
                    "bulk_update"
                }
                .to_owned(),
                outcome: "denied".to_owned(),
                summary: String::new(),
                operation_id: None,
            },
        )
        .await;
        return error_page(
            state,
            headers,
            registry,
            locale,
            StatusCode::FORBIDDEN,
            "forbidden",
            true,
            &user.email,
        )
        .await;
    }
    let Some(raw_ids) = stored.values.get("ids").and_then(Value::as_array) else {
        return error_page(
            state,
            headers,
            registry,
            locale,
            StatusCode::NOT_FOUND,
            "not_found",
            true,
            &user.email,
        )
        .await;
    };
    let mut ids = Vec::new();
    for value in raw_ids {
        let Some(id) = value.as_str().and_then(|text| Uuid::parse_str(text).ok()) else {
            return error_page(
                state,
                headers,
                registry,
                locale,
                StatusCode::UNPROCESSABLE_ENTITY,
                "forbidden",
                true,
                &user.email,
            )
            .await;
        };
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    if ids.is_empty() || ids.len() > action.max_ids {
        return error_page(
            state,
            headers,
            registry,
            locale,
            StatusCode::UNPROCESSABLE_ENTITY,
            "forbidden",
            true,
            &user.email,
        )
        .await;
    }
    let verb = if action.permission == "delete" {
        "delete"
    } else {
        "update"
    };
    let scope = access_scope(model.as_ref(), user, true);
    let applied = {
        let txn = match state.db.begin().await {
            Ok(txn) => txn,
            Err(_) => {
                return error_page(
                    state,
                    headers,
                    registry,
                    locale,
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "bulk_failed",
                    true,
                    &user.email,
                )
                .await;
            }
        };
        async {
            let mut rows = Vec::new();
            for id in &ids {
                match get_record(&txn, model.descriptor().table, model.columns(), *id, &scope).await
                {
                    Ok(Some(row)) if model.object_allows(user, verb, &row) => rows.push((*id, row)),
                    Ok(_) => return Err((StatusCode::NOT_FOUND, "bulk_denied")),
                    Err(_) => return Err((StatusCode::INTERNAL_SERVER_ERROR, "bulk_failed")),
                }
            }
            let audit_action = if matches!(action.operation, ActionOperation::Delete) {
                "bulk_delete"
            } else {
                "bulk_update"
            };
            if let ActionOperation::Custom(custom) = &action.operation {
                let records: Vec<Value> = rows.iter().map(|(_, row)| row.clone()).collect();
                let mut context = crate::actions::ActionContext::prepare(
                    custom.columns,
                    model.columns(),
                    model.meta(),
                    &records,
                );
                if custom.handler.apply(&mut context).await.is_err() {
                    return Err((StatusCode::UNPROCESSABLE_ENTITY, "bulk_failed"));
                }
                for write in context.planned() {
                    let body = match &write.value {
                        crate::actions::ActionValue::Text(text) => json!({ write.column: text }),
                        crate::actions::ActionValue::Integer(number) => {
                            json!({ write.column: number })
                        }
                        crate::actions::ActionValue::Boolean(flag) => json!({ write.column: flag }),
                    };
                    let before = rows
                        .iter()
                        .find(|(id, _)| *id == write.id)
                        .map(|(_, row)| row);
                    let after = match model.update_in(&txn, write.id, body, &scope, user).await {
                        Ok(row) => row,
                        Err(FexuError::Validation(_))
                        | Err(FexuError::Denied(_))
                        | Err(FexuError::NotFound) => {
                            return Err((StatusCode::UNPROCESSABLE_ENTITY, "bulk_failed"));
                        }
                        Err(_) => return Err((StatusCode::INTERNAL_SERVER_ERROR, "bulk_failed")),
                    };
                    let summary = audit::change_summary(model.meta(), before, Some(&after));
                    if audit::insert(
                        &txn,
                        &Event {
                            actor_id: user.id,
                            model_key: model.meta().key.to_owned(),
                            record_id: write.id.to_string(),
                            action: audit_action.to_owned(),
                            outcome: "ok".to_owned(),
                            summary,
                            operation_id: None,
                        },
                    )
                    .await
                    .is_err()
                    {
                        return Err((StatusCode::INTERNAL_SERVER_ERROR, "bulk_failed"));
                    }
                }
                let count = rows.len();
                txn.commit()
                    .await
                    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "bulk_failed"))?;
                return Ok(count);
            }
            for (id, before) in &rows {
                let after = match &action.operation {
                    ActionOperation::Delete => {
                        match delete_record(
                            &txn,
                            model.descriptor().table,
                            *id,
                            &scope,
                            model.columns(),
                        )
                        .await
                        {
                            Ok(1) => None,
                            Ok(_) => return Err((StatusCode::NOT_FOUND, "bulk_denied")),
                            Err(_) => {
                                return Err((StatusCode::INTERNAL_SERVER_ERROR, "bulk_failed"))
                            }
                        }
                    }
                    ActionOperation::SetBool { column, value } => {
                        match update_record(
                            &txn,
                            model.descriptor().table,
                            model.columns(),
                            *id,
                            vec![(*column, DbValue::from(*value))],
                            &scope,
                        )
                        .await
                        {
                            Ok(row) => Some(row),
                            Err(_) => {
                                return Err((StatusCode::INTERNAL_SERVER_ERROR, "bulk_failed"))
                            }
                        }
                    }
                    ActionOperation::Custom(_) => {
                        return Err((StatusCode::INTERNAL_SERVER_ERROR, "bulk_failed"))
                    }
                };
                let summary = audit::change_summary(model.meta(), Some(before), after.as_ref());
                if audit::insert(
                    &txn,
                    &Event {
                        actor_id: user.id,
                        model_key: model.meta().key.to_owned(),
                        record_id: id.to_string(),
                        action: audit_action.to_owned(),
                        outcome: "ok".to_owned(),
                        summary,
                        operation_id: None,
                    },
                )
                .await
                .is_err()
                {
                    return Err((StatusCode::INTERNAL_SERVER_ERROR, "bulk_failed"));
                }
            }
            let count = rows.len();
            txn.commit()
                .await
                .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "bulk_failed"))?;
            Ok(count)
        }
        .await
    };
    let count = match applied {
        Ok(count) => count,
        Err((status, key)) => {
            return error_page(
                state,
                headers,
                registry,
                locale,
                status,
                key,
                true,
                &user.email,
            )
            .await;
        }
    };
    let _ = drafts::delete(&state.db, draft, user.session_id, user.id).await;
    redirect(&format!(
        "/admin/models/{model_key}?notice=changed&changed={count}"
    ))
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn page_as(
    state: &AppState,
    user: &AuthUser,
    registry: &AdminRegistry,
    template: &str,
    locale: &str,
    theme: impl AsRef<str>,
    title: &str,
    signed_in: bool,
    superuser: bool,
    email: &str,
    csrf: &str,
    path: &str,
    notice_text: &str,
    alert: &str,
    mut extra: Value,
) -> Response {
    crate::project::ensure(registry, &state.db).await;
    let navigation = crate::nav::navigation(state, user, registry, locale, path).await;
    let user_name = match state.auth.as_ref() {
        Some(auth) => auth.display_name(user.id).await.unwrap_or_default(),
        None => String::new(),
    };
    if let Some(object) = extra.as_object_mut() {
        object.insert("nav_sections".to_owned(), navigation.sections);
        object.insert("model_groups".to_owned(), navigation.groups);
        object.insert("user_id".to_owned(), json!(user.id.to_string()));
        object
            .entry("user_name".to_owned())
            .or_insert_with(|| json!(user_name));
    }
    page(
        registry,
        template,
        locale,
        theme,
        title,
        signed_in,
        superuser,
        email,
        csrf,
        path,
        notice_text,
        alert,
        extra,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn page(
    registry: &AdminRegistry,
    template: &str,
    locale: &str,
    theme: impl AsRef<str>,
    title: &str,
    signed_in: bool,
    superuser: bool,
    email: &str,
    csrf: &str,
    path: &str,
    notice_text: &str,
    alert: &str,
    extra: Value,
) -> Response {
    let theme = theme.as_ref();
    let branding = registry.branding();
    let icons = registry.icons();
    let mut value = json!({
        "locale": locale,
        "theme": theme,
        "title": title,
        "site_title": branding.site_title,
        "site_header": branding.site_header,
        "app_name": branding.app_name,
        "brand_mark": brand_mark(&branding.site_header),
        "query": "",
        "primary_color": brand_for_ui(&branding.primary_color),
        "project_color": color_input_value(&branding.primary_color),
        "project_currency": branding.currency,
        "project_locale": branding.default_locale,
        "project_fallback": branding.fallback_locale,
        "favicon": icons.favicon,
        "apple_touch_icon": icons.apple_touch_icon,
        "nav_dashboard": template != "error.html" && (path == "/admin" || path == "/admin/"),
        "nav_models": path.starts_with("/admin/models"),
        "nav_audit": path.starts_with("/admin/audit"),
        "nav_users": path.starts_with("/admin/directory/users"),
        "nav_roles": path.starts_with("/admin/directory/roles") && !path.starts_with("/admin/directory/permissions"),
        "nav_permissions": path.starts_with("/admin/directory/permissions"),
        "i18n": catalog(locale),
        "signed_in": signed_in,
        "nav": signed_in,
        "superuser": superuser,
        "user_email": email,
        "user_id": "",
        "user_initial": user_initial(email),
        "csrf_token": csrf,
        "current_path": path,
        "notice": notice_text,
        "alert": alert
    });
    if let (Some(target), Some(source)) = (value.as_object_mut(), extra.as_object()) {
        for (key, item) in source {
            target.insert(key.clone(), item.clone());
        }
    }
    if let Some(name) = value.get("user_name").and_then(Value::as_str) {
        if !name.is_empty() {
            let initial = user_initial(name);
            if let Some(target) = value.as_object_mut() {
                target.insert("user_initial".to_owned(), json!(initial));
            }
        }
    }
    if value.get("nav_custom").is_none() {
        if let Some(target) = value.as_object_mut() {
            target.insert("nav_custom".to_owned(), Value::Array(Vec::new()));
        }
    }
    if value.get("nav_sections").is_none() {
        if let Some(target) = value.as_object_mut() {
            target.insert("nav_sections".to_owned(), Value::Array(Vec::new()));
        }
    }
    if value.get("model_groups").is_none() {
        if let Some(target) = value.as_object_mut() {
            target.insert("model_groups".to_owned(), Value::Array(Vec::new()));
        }
    }
    match render(template, value, registry.templates()) {
        Ok(body) => Html(body).into_response(),
        Err(error) => {
            tracing::error!(%error, "admin template failed");
            plain(StatusCode::INTERNAL_SERVER_ERROR, "Request rejected")
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn error_page(
    state: &AppState,
    headers: &HeaderMap,
    registry: &AdminRegistry,
    locale: &str,
    status: StatusCode,
    key: &str,
    signed_in: bool,
    email: &str,
) -> Response {
    let message = catalog(locale)
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("Error")
        .to_owned();
    let title = catalog(locale)
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or("Error")
        .to_owned();
    let csrf = if signed_in {
        match state.auth.as_ref() {
            Some(auth) => session_csrf(auth, headers).await,
            None => String::new(),
        }
    } else {
        String::new()
    };
    let signed_in = signed_in && !csrf.is_empty();
    let actor = match state.auth.as_ref() {
        Some(auth) if signed_in => auth.authenticate(headers, &Method::GET).await.ok(),
        _ => None,
    };
    let show_nav = actor.is_some() && key != "staff_required";
    let superuser = actor.as_ref().is_some_and(|user| user.is_superuser);
    let mut response = if let Some(user) = actor.as_ref() {
        page_as(
            state,
            user,
            registry,
            "error.html",
            locale,
            theme_of(headers),
            &title,
            true,
            superuser,
            email,
            &csrf,
            "/admin",
            "",
            "",
            json!({ "message": message, "nav": show_nav }),
        )
        .await
    } else {
        page(
            registry,
            "error.html",
            locale,
            theme_of(headers),
            &title,
            false,
            false,
            email,
            &csrf,
            "/admin",
            "",
            "",
            json!({ "message": message, "nav": false }),
        )
    };
    *response.status_mut() = status;
    response
}

fn redirect(location: &str) -> Response {
    let mut response = StatusCode::SEE_OTHER.into_response();
    if let Ok(value) = HeaderValue::from_str(location) {
        response.headers_mut().insert(header::LOCATION, value);
    }
    response
}

fn plain(status: StatusCode, message: &str) -> Response {
    (status, message.to_owned()).into_response()
}

struct Html(String);
impl IntoResponse for Html {
    fn into_response(self) -> Response {
        let mut response = Response::new(axum::body::Body::from(self.0));
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/html; charset=utf-8"),
        );
        response.headers_mut().insert(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        );
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        response
    }
}

pub(crate) fn locale_of(
    headers: &HeaderMap,
    default_locale: &str,
    fallback_locale: &str,
) -> String {
    let requested = cookie(headers, "fexu_locale")
        .or_else(|| accept_language(headers))
        .unwrap_or_default();
    normalize(&requested, default_locale, fallback_locale)
}

pub(crate) fn theme_of(headers: &HeaderMap) -> String {
    match cookie(headers, "fexu_theme").as_deref() {
        Some("light") => "light".to_owned(),
        Some("dark") => "dark".to_owned(),
        _ => "system".to_owned(),
    }
}

fn accept_language(headers: &HeaderMap) -> Option<String> {
    let value = headers.get(header::ACCEPT_LANGUAGE)?.to_str().ok()?;
    if value.to_ascii_lowercase().contains("de") {
        Some("de".to_owned())
    } else if value.to_ascii_lowercase().contains("en") {
        Some("en".to_owned())
    } else {
        None
    }
}

pub(crate) async fn session_csrf(auth: &fexu_auth::AuthService, headers: &HeaderMap) -> String {
    auth.csrf_from_request(headers).await.unwrap_or_default()
}

fn login_token(headers: &HeaderMap) -> String {
    cookie(headers, "fexu_login_csrf")
        .filter(|value| valid_login_csrf(value))
        .unwrap_or_else(random_token)
}

fn login_gate(
    auth: &fexu_auth::AuthService,
    headers: &HeaderMap,
    supplied: &str,
) -> Result<(), &'static str> {
    if auth.origin_allowed(headers, &Method::POST).is_err() {
        return Err("origin_rejected");
    }
    if supplied.is_empty() {
        return Err("form_token_missing");
    }
    let Some(expected) = cookie(headers, "fexu_login_csrf") else {
        return Err("login_csrf_missing");
    };
    if !valid_login_csrf(&expected) || !constant_eq(&expected, supplied) {
        return Err("login_csrf_mismatch");
    }
    Ok(())
}

fn valid_login_csrf(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|char| char.is_ascii_hexdigit())
}

fn log_login_rejection(headers: &HeaderMap, reason: &'static str) {
    if reason != "origin_rejected" {
        tracing::warn!(reason, "admin login rejected");
        return;
    }
    let (source, class, port) = presented_origin_shape(headers);
    tracing::warn!(reason, source, class, port, "admin login rejected");
}

fn presented_origin_shape(headers: &HeaderMap) -> (&'static str, &'static str, u16) {
    let (source, raw) =
        if let Some(value) = headers.get("origin").and_then(|value| value.to_str().ok()) {
            ("origin", value)
        } else if let Some(value) = headers.get("referer").and_then(|value| value.to_str().ok()) {
            ("referer", value)
        } else {
            return ("absent", "absent", 0);
        };
    let value = raw.trim();
    if value.eq_ignore_ascii_case("null") {
        return (source, "null", 0);
    }
    let Some((scheme, rest)) = value.split_once("://") else {
        return (source, "malformed", 0);
    };
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return (source, "malformed", 0);
    }
    let hostport = rest.split(['/', '?', '#', ' ', '@']).next().unwrap_or("");
    let (host, port) = if let Some(rest) = hostport.strip_prefix('[') {
        let Some((host, port)) = rest.split_once("]:") else {
            return (source, "malformed", 0);
        };
        (host, port)
    } else if let Some((host, port)) = hostport.rsplit_once(':') {
        (host, port)
    } else {
        (
            hostport,
            if scheme.eq_ignore_ascii_case("https") {
                "443"
            } else {
                "80"
            },
        )
    };
    let class = if host.eq_ignore_ascii_case("localhost") {
        "localhost"
    } else if host == "127.0.0.1" || host == "::1" {
        "loopback"
    } else if host.is_empty() {
        "malformed"
    } else {
        "other"
    };
    (source, class, port.parse().unwrap_or(0))
}

fn random_token() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
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

/// The login page submits its actual origin on same-origin POST requests.
/// `no-referrer` could make browsers send `Origin: null`. Recovery pages
/// set their own policy and are not modified here.
fn same_origin_document(response: &mut Response) {
    response.headers_mut().insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("same-origin"),
    );
}

fn login_cookie(config: Option<&AuthConfig>, token: &str) -> String {
    let secure = config.is_some_and(|config| config.cookie_secure);
    let same = match config.map(|config| config.same_site) {
        Some(CookieSameSite::Strict) => "Strict",
        _ => "Lax",
    };
    format!(
        "fexu_login_csrf={token}; HttpOnly; Path=/; SameSite={same}; Max-Age=600{}",
        if secure { "; Secure" } else { "" }
    )
}

fn preference_cookie(name: &str, value: &str, config: Option<&AuthConfig>) -> String {
    let secure = config.is_some_and(|config| config.cookie_secure);
    let same = match config.map(|config| config.same_site) {
        Some(CookieSameSite::Strict) => "Strict",
        _ => "Lax",
    };
    format!(
        "{name}={value}; HttpOnly; Path=/; SameSite={same}; Max-Age=31536000{}",
        if secure { "; Secure" } else { "" }
    )
}

fn clear_cookie(name: &str, config: &AuthConfig) -> String {
    preference_cookie(name, "", Some(config)).replace("Max-Age=31536000", "Max-Age=0")
}

fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find_map(|raw| {
            raw.split(';').find_map(|part| {
                let (key, value) = part.trim().split_once('=')?;
                (key == name).then(|| value.trim().to_owned())
            })
        })
}

fn append_cookie(response: &mut Response, value: &str) {
    if let Ok(header_value) = HeaderValue::from_str(value) {
        response
            .headers_mut()
            .append(header::SET_COOKIE, header_value);
    }
}

pub(crate) fn csrf_token_from(headers: &HeaderMap, body: &Bytes) -> String {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    if content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .eq_ignore_ascii_case("multipart/form-data")
    {
        return fexu_storage::multipart_text_field(content_type, body, "csrf_token")
            .map(|value| value.trim().to_owned())
            .unwrap_or_default();
    }
    parse_form(body)
        .get("csrf_token")
        .cloned()
        .unwrap_or_default()
}

fn row_form(body: &Bytes) -> Result<HashMap<String, String>, ()> {
    let mut values = HashMap::new();
    for pair in String::from_utf8_lossy(body).split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if values.insert(decode(key), decode(value)).is_some() {
            return Err(());
        }
    }
    Ok(values)
}

fn apply_row_fields(
    object: &mut Map<String, Value>,
    columns: &[ColumnSpec],
    allowed: &[&str],
    form: &HashMap<String, String>,
) -> Result<(), ()> {
    for key in form.keys() {
        if key == "csrf_token" {
            continue;
        }
        if !allowed.contains(&key.as_str()) {
            return Err(());
        }
    }
    for field in allowed {
        let Some(posted) = form.get(*field) else {
            continue;
        };
        let Some(column) = columns.iter().find(|column| column.name == *field) else {
            continue;
        };
        object.insert((*field).to_owned(), typed_value(column, posted));
    }
    Ok(())
}

pub(crate) fn parse_form(body: &Bytes) -> HashMap<String, String> {
    let mut values = HashMap::new();
    for pair in String::from_utf8_lossy(body).split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        values.insert(decode(key), decode(value));
    }
    values
}

fn decode(value: &str) -> String {
    let mut bytes = Vec::new();
    let input = value.as_bytes();
    let mut index = 0;
    while index < input.len() {
        match input[index] {
            b'+' => {
                bytes.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < input.len() => {
                let hex = &value[index + 1..index + 3];
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    bytes.push(byte);
                    index += 3;
                } else {
                    bytes.push(b'%');
                    index += 1;
                }
            }
            byte => {
                bytes.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn safe_next(value: &str) -> Option<&str> {
    if value.starts_with("/admin")
        && !value.starts_with("//")
        && !value.contains('\\')
        && !value.contains("://")
        && !value.chars().any(|char| char.is_control())
        && value.len() <= 200
    {
        Some(value)
    } else {
        None
    }
}

fn sort_query(params: &HashMap<String, String>, sort: &str, dir: &str) -> String {
    let mut pairs = vec![
        format!(
            "q={}",
            url_encode(params.get("q").map(String::as_str).unwrap_or(""))
        ),
        format!("sort={sort}"),
        format!("dir={dir}"),
    ];
    let mut keys: Vec<&String> = params
        .keys()
        .filter(|key| key.starts_with("filter."))
        .collect();
    keys.sort();
    for key in keys {
        pairs.push(format!(
            "{}={}",
            url_encode(key),
            url_encode(params.get(key).map(String::as_str).unwrap_or(""))
        ));
    }
    pairs.join("&")
}

fn is_uuid(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 36
        && bytes[8] == b'-'
        && bytes[13] == b'-'
        && bytes[18] == b'-'
        && bytes[23] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| matches!(index, 8 | 13 | 18 | 23) || byte.is_ascii_hexdigit())
}

fn shorten_uuid(value: &str) -> String {
    format!("{}…{}", &value[..8], &value[value.len() - 4..])
}

fn color_input_value(hex: &str) -> String {
    let body = hex.trim().trim_start_matches('#').to_ascii_lowercase();
    if body.len() == 3 {
        let chars: Vec<char> = body.chars().collect();
        return format!(
            "#{}{}{}{}{}{}",
            chars[0], chars[0], chars[1], chars[1], chars[2], chars[2]
        );
    }
    format!("#{body}")
}

/// Use the project color only if white text achieves a contrast ratio of at least 4.5:1.
fn brand_for_ui(hex: &str) -> String {
    if contrast_ratio(hex, "#ffffff") >= 4.5 {
        hex.to_owned()
    } else {
        "#0f172a".to_owned()
    }
}

fn contrast_ratio(left: &str, right: &str) -> f64 {
    let Some(left) = relative_luminance(left) else {
        return 1.0;
    };
    let Some(right) = relative_luminance(right) else {
        return 1.0;
    };
    let (hi, lo) = if left > right {
        (left, right)
    } else {
        (right, left)
    };
    (hi + 0.05) / (lo + 0.05)
}

fn relative_luminance(hex: &str) -> Option<f64> {
    let (red, green, blue) = rgb(hex)?;
    Some(0.2126 * channel_lin(red) + 0.7152 * channel_lin(green) + 0.0722 * channel_lin(blue))
}

fn rgb(hex: &str) -> Option<(u8, u8, u8)> {
    let hex = hex.strip_prefix('#')?;
    let expanded;
    let hex = if hex.len() == 3 {
        expanded = hex.chars().fold(String::new(), |mut output, char| {
            output.push(char);
            output.push(char);
            output
        });
        expanded.as_str()
    } else {
        hex
    };
    if hex.len() != 6 {
        return None;
    }
    Some((
        u8::from_str_radix(&hex[0..2], 16).ok()?,
        u8::from_str_radix(&hex[2..4], 16).ok()?,
        u8::from_str_radix(&hex[4..6], 16).ok()?,
    ))
}

fn channel_lin(channel: u8) -> f64 {
    let value = f64::from(channel) / 255.0;
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

const RECORD_HISTORY_LIMIT: usize = 8;

struct RecordHistory {
    events: Vec<Value>,
    more: bool,
}

impl RecordHistory {
    fn none() -> Self {
        Self {
            events: Vec::new(),
            more: false,
        }
    }
}

async fn record_history(
    state: &AppState,
    user: &AuthUser,
    locale: &str,
    model_key: &str,
    record_id: &str,
) -> RecordHistory {
    if model_key.is_empty() || record_id.is_empty() {
        return RecordHistory::none();
    }
    let actor = (!user.is_superuser).then_some(user.id);
    let Ok((events, has_more)) =
        audit::matching(&state.db, actor, Some(model_key), Some(record_id), 0).await
    else {
        return RecordHistory::none();
    };
    let more = has_more || events.len() > RECORD_HISTORY_LIMIT;
    let events: Vec<_> = events.into_iter().take(RECORD_HISTORY_LIMIT).collect();
    let mut emails = std::collections::HashMap::<uuid::Uuid, String>::new();
    if user.is_superuser {
        if let Some(auth) = state.auth.as_ref() {
            for event in &events {
                if event.actor_id == user.id || emails.contains_key(&event.actor_id) {
                    continue;
                }
                if let Ok(Some(account)) = auth.get_user(event.actor_id).await {
                    emails.insert(event.actor_id, account.email);
                }
            }
        }
    }
    RecordHistory {
        events: render_audit_events(user, locale, &emails, events),
        more,
    }
}

const OVERVIEW_LIMIT: usize = 6;

async fn overview_extra(state: &AppState, user: &AuthUser, locale: &str) -> Value {
    let actor = (!user.is_superuser).then_some(user.id);
    let listed = audit::page(&state.db, actor, 0).await;
    let (events, activity_error) = match listed {
        Ok((events, _)) => {
            let events: Vec<_> = events.into_iter().take(OVERVIEW_LIMIT).collect();
            (events, false)
        }
        Err(_) => (Vec::new(), true),
    };
    let mut emails = std::collections::HashMap::<uuid::Uuid, String>::new();
    if user.is_superuser && !activity_error {
        if let Some(auth) = state.auth.as_ref() {
            for event in &events {
                if event.actor_id == user.id || emails.contains_key(&event.actor_id) {
                    continue;
                }
                if let Ok(Some(account)) = auth.get_user(event.actor_id).await {
                    emails.insert(event.actor_id, account.email);
                }
            }
        }
    }
    json!({
        "user_email": user.email,
        "activity": render_audit_events(user, locale, &emails, events),
        "activity_error": activity_error,
    })
}

fn render_audit_events(
    user: &AuthUser,
    locale: &str,
    emails: &std::collections::HashMap<uuid::Uuid, String>,
    events: Vec<audit::StoredEvent>,
) -> Vec<Value> {
    events
        .into_iter()
        .map(|event| {
            let actor_label = if event.actor_id == user.id {
                user.email.clone()
            } else if user.is_superuser {
                emails
                    .get(&event.actor_id)
                    .cloned()
                    .unwrap_or_else(|| event.actor_id.to_string())
            } else {
                event.actor_id.to_string()
            };
            let formatted = audit::format_unix_utc(event.created_at);
            json!({
                "when": formatted.as_ref().map(|(label, _)| label.clone()).unwrap_or_default(),
                "when_iso": formatted.as_ref().map(|(_, iso)| iso.clone()).unwrap_or_default(),
                "actor": actor_label,
                "action": event.action,
                "action_label": action_label(locale, &event.action),
                "outcome": event.outcome,
                "summary": event.summary,
            })
        })
        .collect()
}

fn brand_mark(name: &str) -> String {
    name.chars()
        .find(|char| char.is_alphanumeric())
        .map(|char| char.to_uppercase().collect::<String>())
        .unwrap_or_else(|| "F".to_owned())
}

fn user_initial(email: &str) -> String {
    email
        .chars()
        .find(|char| char.is_alphanumeric())
        .map(|char| char.to_uppercase().collect::<String>())
        .unwrap_or_else(|| "?".to_owned())
}

fn action_label(locale: &str, action: &str) -> String {
    let key = match action {
        "create" => "audit_create",
        "update" => "audit_update",
        "delete" => "audit_delete",
        "bulk_update" => "audit_bulk_update",
        "bulk_delete" => "audit_bulk_delete",
        "user_create" => "audit_user_create",
        "user_update" => "audit_user_update",
        "password_reset" => "audit_password_reset",
        "role_assign" => "audit_role_assign",
        "role_revoke" => "audit_role_revoke",
        "permission_grant" => "audit_permission_grant",
        "permission_revoke" => "audit_permission_revoke",
        _ => return action.to_owned(),
    };
    catalog(locale)
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or(action)
        .to_owned()
}

fn history_href(model: &str, record: &str) -> String {
    format!(
        "/admin/audit?model={}&record={}",
        url_encode(model),
        url_encode(record)
    )
}

fn audit_key(value: &str) -> Option<String> {
    let ok = !value.is_empty()
        && value.len() <= 80
        && value
            .chars()
            .all(|char| char.is_ascii_alphanumeric() || matches!(char, '_' | '-' | '.'));
    ok.then(|| value.to_owned())
}

fn audit_page_href(page: u32, model: Option<&str>, record: Option<&str>) -> String {
    let mut href = format!("/admin/audit?page={page}");
    if let Some(model) = model {
        href.push_str("&model=");
        href.push_str(&url_encode(model));
    }
    if let Some(record) = record {
        href.push_str("&record=");
        href.push_str(&url_encode(record));
    }
    href
}

fn url_encode(value: &str) -> String {
    let mut output = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.') {
            output.push(byte as char);
        } else {
            output.push_str(&format!("%{byte:02X}"));
        }
    }
    output
}

struct DateNavigation {
    bounds: Vec<(&'static str, bool, DbValue)>,
    view: Value,
}

#[allow(clippy::too_many_arguments)]
async fn date_navigation(
    state: &AppState,
    model: &dyn ManagedModel,
    locale: &str,
    params: &HashMap<String, String>,
    search_columns: &[&str],
    search: Option<&str>,
    filters: &[(&str, DbValue)],
    scope: &fexu_core::QueryScope,
    list_path: &str,
) -> Result<DateNavigation, ()> {
    let Some(column) = model.meta().date_hierarchy else {
        return Ok(DateNavigation {
            bounds: Vec::new(),
            view: Value::Null,
        });
    };
    let kind = model
        .columns()
        .iter()
        .find(|item| item.name == column)
        .map(|item| item.kind)
        .ok_or(())?;
    let year = optional_number(params, "year")?;
    let month = optional_number(params, "month")?;
    let day = optional_number(params, "day")?;
    if month.is_some() && year.is_none() || day.is_some() && month.is_none() {
        return Err(());
    }
    let mut bounds = Vec::new();
    if let Some(year) = year {
        if !(1..=9999).contains(&year) {
            return Err(());
        }
        let bound = fexu_core::hierarchy_bound(year as i32, month, day).map_err(|_| ())?;
        let (start, end) = match kind {
            ColumnKind::Date => (DbValue::from(bound.start), DbValue::from(bound.end)),
            ColumnKind::DateTime => (
                DbValue::from(bound.start.and_hms_opt(0, 0, 0).ok_or(())?.and_utc()),
                DbValue::from(bound.end.and_hms_opt(0, 0, 0).ok_or(())?.and_utc()),
            ),
            _ => return Err(()),
        };
        bounds.push((column, true, start));
        bounds.push((column, false, end));
    }
    let width = if year.is_none() {
        4
    } else if month.is_none() {
        7
    } else if day.is_none() {
        10
    } else {
        0
    };
    let buckets = if width == 0 {
        Vec::new()
    } else {
        date_buckets(
            &state.db,
            model.descriptor().table,
            model.columns(),
            column,
            width,
            search_columns,
            search,
            filters,
            &bounds,
            scope,
        )
        .await
        .map_err(|_| ())?
    };
    let root = hierarchy_href(list_path, params, None, None, None);
    let mut crumbs = Vec::new();
    if let Some(year) = year {
        crumbs.push(json!({
            "label": year.to_string(),
            "href": hierarchy_href(list_path, params, Some(year), None, None)
        }));
    }
    if let (Some(year), Some(month)) = (year, month) {
        crumbs.push(json!({
            "label": month_name(locale, month),
            "href": hierarchy_href(list_path, params, Some(year), Some(month), None)
        }));
    }
    let items: Vec<Value> = buckets
        .into_iter()
        .filter_map(|(label, count)| {
            let (text, href) = match width {
                4 => {
                    let value = label.parse::<u32>().ok()?;
                    (
                        label,
                        hierarchy_href(list_path, params, Some(value), None, None),
                    )
                }
                7 => {
                    let month = label.get(5..7)?.parse::<u32>().ok()?;
                    (
                        month_name(locale, month),
                        hierarchy_href(list_path, params, year, Some(month), None),
                    )
                }
                _ => {
                    let day = label.get(8..10)?.parse::<u32>().ok()?;
                    (
                        day.to_string(),
                        hierarchy_href(list_path, params, year, month, Some(day)),
                    )
                }
            };
            Some(json!({ "label": text, "href": href, "count": count }))
        })
        .collect();
    let field = model
        .meta()
        .fields
        .iter()
        .find(|field| field.name == column)
        .map(|field| field.label)
        .unwrap_or(column);
    Ok(DateNavigation {
        bounds,
        view: json!({
            "field": field,
            "root": root,
            "crumbs": crumbs,
            "items": items
        }),
    })
}

fn optional_number(params: &HashMap<String, String>, key: &str) -> Result<Option<u32>, ()> {
    match params.get(key).map(String::as_str) {
        None | Some("") => Ok(None),
        Some(value) => value.parse::<u32>().map(Some).map_err(|_| ()),
    }
}

fn month_name(locale: &str, month: u32) -> String {
    catalog(locale)
        .get(format!("month_{month}"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()
}

fn hierarchy_href(
    list_path: &str,
    params: &HashMap<String, String>,
    year: Option<u32>,
    month: Option<u32>,
    day: Option<u32>,
) -> String {
    let mut pairs = Vec::new();
    for (key, value) in params {
        if matches!(
            key.as_str(),
            "page" | "notice" | "changed" | "year" | "month" | "day"
        ) {
            continue;
        }
        pairs.push(format!("{}={}", url_encode(key), url_encode(value)));
    }
    if let Some(year) = year {
        pairs.push(format!("year={year}"));
    }
    if let Some(month) = month {
        pairs.push(format!("month={month}"));
    }
    if let Some(day) = day {
        pairs.push(format!("day={day}"));
    }
    if pairs.is_empty() {
        list_path.to_owned()
    } else {
        format!("{list_path}?{}", pairs.join("&"))
    }
}

fn normalize_temporal(
    columns: &[ColumnSpec],
    object: &mut Map<String, Value>,
) -> Result<(), (String, &'static str)> {
    for column in columns {
        let Some(value) = object.get(column.name).cloned() else {
            continue;
        };
        if value.is_null() {
            continue;
        }
        let Some(text) = value.as_str() else {
            continue;
        };
        if text.is_empty() {
            if column.nullable {
                object.insert(column.name.to_owned(), Value::Null);
            }
            continue;
        }
        let canonical = match column.kind {
            ColumnKind::Date => fexu_core::parse_date(text)
                .map(fexu_core::temporal::format_date)
                .map_err(|code| (column.name.to_owned(), code))?,
            ColumnKind::Time => fexu_core::parse_time(text)
                .map(fexu_core::temporal::format_time)
                .map_err(|code| (column.name.to_owned(), code))?,
            ColumnKind::DateTime => fexu_core::parse_datetime_local(text)
                .or_else(|_| fexu_core::parse_datetime_utc(text))
                .map(fexu_core::temporal::format_datetime)
                .map_err(|code| (column.name.to_owned(), code))?,
            ColumnKind::Json => fexu_core::canonical_json_text(text)
                .map_err(|code| (column.name.to_owned(), code))?,
            _ => continue,
        };
        object.insert(column.name.to_owned(), json!(canonical));
    }
    Ok(())
}

fn flash(locale: &str, code: &str) -> String {
    notice(locale, code).unwrap_or_default()
}

fn preserve_query(params: &HashMap<String, String>, _page: u32) -> String {
    let mut pairs = Vec::new();
    for (key, value) in params {
        if key == "page" || key == "notice" {
            continue;
        }
        pairs.push(format!("{}={}", url_encode(key), url_encode(value)));
    }
    pairs.join("&")
}

fn form_flag(form: &HashMap<String, String>, name: &str) -> Option<bool> {
    match form.get(name).map(String::as_str) {
        Some("true") | Some("1") => Some(true),
        Some("false") | Some("0") => Some(false),
        _ => None,
    }
}

fn role_label_ok(value: &str) -> bool {
    let value = value.trim();
    (1..=80).contains(&value.chars().count())
        && value.chars().all(|character| {
            character.is_alphanumeric() || matches!(character, ' ' | '_' | '.' | '-')
        })
}

fn permission_catalog(registry: &AdminRegistry, locale: &str) -> Vec<Value> {
    let mut rows: Vec<(String, String, String)> = Vec::new();
    for model in registry.models() {
        let Some(code) = model.meta().permission else {
            continue;
        };
        if !identifier(code) {
            continue;
        }
        let app_key = registry.model_app(model.meta().key).unwrap_or("");
        let app = registry
            .app_catalog()
            .iter()
            .find(|(key, _)| key == app_key)
            .map(|(_, label)| label.clone())
            .unwrap_or_else(|| app_key.to_owned());
        let label = model.meta().label.to_owned();
        if let Some(existing) = rows.iter_mut().find(|(key, _, _)| key == code) {
            if label.chars().count() < existing.2.chars().count() {
                existing.1 = app;
                existing.2 = label;
            }
            continue;
        }
        rows.push((code.to_owned(), app, label));
    }
    rows.sort_by(|left, right| left.1.cmp(&right.1).then(left.2.cmp(&right.2)));
    let mut values: Vec<Value> = rows
        .into_iter()
        .map(|(key, app, label)| {
            json!({
                "key": key,
                "app": app,
                "label": label,
                "offer_create": true,
                "offer_read": true,
                "offer_update": true,
                "offer_delete": true
            })
        })
        .collect();
    let roles = catalog(locale)
        .get("roles_grant")
        .and_then(Value::as_str)
        .unwrap_or("Role assignment")
        .to_owned();
    values.push(json!({
        "key": "roles",
        "app": "",
        "label": roles,
        "offer_create": false,
        "offer_read": false,
        "offer_update": true,
        "offer_delete": false
    }));
    values
}

async fn save_permission_matrix(
    state: &AppState,
    headers: &HeaderMap,
    registry: &AdminRegistry,
    locale: &str,
    user: &AuthUser,
    role: &str,
    form: &HashMap<String, String>,
) -> Response {
    let auth = state.auth.as_ref().unwrap();
    let listed = auth.list_roles().await.unwrap_or_default();
    let resolved = if let Ok(id) = uuid::Uuid::parse_str(role) {
        listed.iter().find(|(role_id, _)| role_id == &id).cloned()
    } else if role_label_ok(role) {
        listed.iter().find(|(_, name)| name == role).cloned()
    } else {
        None
    };
    let Some((role_id, role_name)) = resolved else {
        return error_page(
            state,
            headers,
            registry,
            locale,
            StatusCode::UNPROCESSABLE_ENTITY,
            "unknown_role",
            true,
            &user.email,
        )
        .await;
    };
    let granted = auth
        .list_role_permissions(&role_name)
        .await
        .unwrap_or_default();
    for row in permission_catalog(registry, locale) {
        let key = row.get("key").and_then(Value::as_str).unwrap_or("");
        for action in ["create", "read", "update", "delete"] {
            let offer_key = format!("offer_{action}");
            let offer = row
                .get(&offer_key)
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if !offer || !identifier(key) {
                continue;
            }
            let code = format!("{key}.{action}");
            let wanted = form.contains_key(&code);
            let has = granted.iter().any(|item| item == &code);
            if wanted == has {
                continue;
            }
            if crate::directory::grant_permission(auth, user.id, &role_name, &code, wanted)
                .await
                .is_err()
            {
                return error_page(
                    state,
                    headers,
                    registry,
                    locale,
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "forbidden",
                    true,
                    &user.email,
                )
                .await;
            }
        }
    }
    if form.get("return_to").map(String::as_str) == Some("record") {
        redirect(&format!("/admin/directory/roles/{role_id}?notice=updated"))
    } else {
        redirect("/admin/directory/permissions?notice=updated")
    }
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .chars()
            .all(|char| char.is_ascii_alphanumeric() || matches!(char, '_' | '.'))
}

#[cfg(test)]
mod contrast_tests {
    use super::{brand_for_ui, contrast_ratio, is_uuid};

    #[test]
    fn pale_brand_falls_back_to_slate() {
        assert_eq!(brand_for_ui("#ffffff"), "#0f172a");
        assert_eq!(brand_for_ui("#f4f7fb"), "#0f172a");
    }

    #[test]
    fn readable_brand_keeps_its_hex() {
        assert_eq!(brand_for_ui("#0f172a"), "#0f172a");
        assert!(contrast_ratio("#ffffff", "#9f1239") >= 4.5);
        assert_eq!(brand_for_ui("#9f1239"), "#9f1239");
    }

    #[test]
    fn uuids_are_not_ordinary_labels() {
        assert!(is_uuid("11111111-1111-1111-1111-111111111111"));
        assert!(!is_uuid("Apfel"));
    }
}

#[cfg(test)]
mod relation_label_tests {
    use super::{present_cell, RelationLabels};
    use fexu_core::{ColumnKind, ColumnSpec, ReferentialAction};
    use serde_json::json;
    use std::collections::HashMap;

    fn product_column() -> ColumnSpec {
        ColumnSpec {
            name: "product_id",
            kind: ColumnKind::Uuid,
            nullable: false,
            primary_key: false,
            default_bool: None,
            unique: false,
            indexed: true,
            foreign_table: Some("products"),
            foreign_column: Some("id"),
            on_delete: Some(ReferentialAction::Restrict),
            precision: 0,
            scale: 0,
            choices: &[],
        }
    }

    fn relations(label: Option<&str>) -> HashMap<String, RelationLabels> {
        let id = "11111111-1111-1111-1111-111111111111";
        let mut labels = HashMap::new();
        if let Some(label) = label {
            labels.insert(id.to_owned(), label.to_owned());
        }
        let mut relations = HashMap::new();
        relations.insert(
            "products".to_owned(),
            RelationLabels {
                model_key: if label.is_some() { "products" } else { "" },
                labels,
            },
        );
        relations
    }

    #[test]
    fn visible_target_uses_the_name_and_admin_link() {
        let id = "11111111-1111-1111-1111-111111111111";
        let mut lead = false;
        let cell = present_cell(
            "de",
            "EUR",
            &[product_column()],
            "product_id",
            &json!(id),
            &relations(Some("Apfel")),
            false,
            &mut lead,
        );
        assert_eq!(cell["text"], "Apfel");
        assert_eq!(cell["href"], format!("/admin/models/products/{id}"));
        assert_ne!(cell["text"].as_str(), Some(id));
    }

    #[test]
    fn missing_and_hidden_targets_share_one_label_without_a_link() {
        let id = "11111111-1111-1111-1111-111111111111";
        let columns = [product_column()];
        let hidden = relations(None);
        let mut readable = relations(Some("Apfel"));
        readable.get_mut("products").unwrap().labels.remove(id);
        let mut lead = false;
        let hidden_cell = present_cell(
            "de",
            "EUR",
            &columns,
            "product_id",
            &json!(id),
            &hidden,
            false,
            &mut lead,
        );
        lead = false;
        let missing_cell = present_cell(
            "de",
            "EUR",
            &columns,
            "product_id",
            &json!(id),
            &readable,
            false,
            &mut lead,
        );
        assert_eq!(hidden_cell["text"], "Nicht sichtbar");
        assert_eq!(missing_cell["text"], hidden_cell["text"]);
        assert_eq!(hidden_cell["href"], "");
        assert_eq!(missing_cell["href"], "");
        assert_ne!(hidden_cell["text"].as_str(), Some(id));
    }
}

#[cfg(test)]
mod posted_body_tests {
    use super::{apply_row_fields, read_model_body, row_form};
    use axum::body::Bytes;
    use axum::http::{header, HeaderMap, HeaderValue};
    use fexu_core::{ColumnKind, ColumnSpec};
    use serde_json::{json, Map};

    fn text_column(name: &'static str) -> ColumnSpec {
        ColumnSpec {
            name,
            kind: ColumnKind::Text,
            nullable: true,
            primary_key: false,
            default_bool: None,
            unique: false,
            indexed: false,
            foreign_table: None,
            foreign_column: None,
            on_delete: None,
            precision: 0,
            scale: 0,
            choices: &[],
        }
    }

    #[test]
    fn missing_list_fields_keep_their_stored_value() {
        let columns = [text_column("street"), text_column("city")];
        let mut object = Map::new();
        object.insert("street".into(), json!("Lindengasse"));
        object.insert("city".into(), json!("Wien"));
        let form = row_form(&Bytes::from_static(b"csrf_token=abc&street=Neu")).unwrap();
        apply_row_fields(&mut object, &columns, &["street", "city"], &form).unwrap();
        assert_eq!(object["street"], json!("Neu"));
        assert_eq!(object["city"], json!("Wien"));

        let mut untouched = object.clone();
        let unknown = row_form(&Bytes::from_static(b"csrf_token=abc&street=Weg&note=x")).unwrap();
        assert!(apply_row_fields(&mut untouched, &columns, &["street", "city"], &unknown).is_err());
        assert_eq!(untouched["street"], json!("Neu"));
        assert_eq!(untouched["city"], json!("Wien"));
        assert!(row_form(&Bytes::from_static(b"city=Wien&city=Graz")).is_err());
    }

    #[test]
    fn duplicate_multipart_names_are_rejected() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("multipart/form-data; boundary=bound"),
        );
        let duplicate_text = Bytes::from(
            "--bound\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\nAlt\r\n\
--bound\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\nNeu\r\n\
--bound--\r\n",
        );
        assert!(read_model_body(&headers, &duplicate_text).is_err());
        let duplicate_file = Bytes::from(
            "--bound\r\nContent-Disposition: form-data; name=\"file.cover\"; filename=\"a.gif\"\r\n\r\nGIF89a\r\n\
--bound\r\nContent-Disposition: form-data; name=\"file.cover\"; filename=\"b.gif\"\r\n\r\nGIF89a\r\n\
--bound--\r\n",
        );
        assert!(read_model_body(&headers, &duplicate_file).is_err());
        let checked = Bytes::from(
            "--bound\r\nContent-Disposition: form-data; name=\"active\"\r\n\r\nfalse\r\n\
--bound\r\nContent-Disposition: form-data; name=\"active\"\r\n\r\ntrue\r\n\
--bound\r\nContent-Disposition: form-data; name=\"m2m.tags\"\r\n\r\n11111111-1111-4111-8111-111111111111\r\n\
--bound\r\nContent-Disposition: form-data; name=\"m2m.tags\"\r\n\r\n22222222-2222-4222-8222-222222222222\r\n\
--bound--\r\n",
        );
        let parsed = read_model_body(&headers, &checked).unwrap();
        assert_eq!(
            parsed.fields.get("active").map(String::as_str),
            Some("true")
        );
        assert_eq!(parsed.repeated.get("m2m.tags").map(Vec::len), Some(2));
        let urlencoded = Bytes::from_static(b"title=Alt&title=Neu");
        let mut plain = HeaderMap::new();
        plain.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/x-www-form-urlencoded"),
        );
        assert!(read_model_body(&plain, &urlencoded).is_err());
    }
}

#[cfg(test)]
mod store_rollback_tests {
    use super::commit_model;
    use crate::compose::{save_with, SaveTail, StoredFile, StoredLink};
    use crate::files::{self, fail_next_save};
    use crate::registry::ManagedModel;
    use fexu_auth::{AuthConfig, AuthService, AuthUser};
    use fexu_core::{
        AdminFieldMeta, AdminInline, AdminModelMeta, ColumnKind, ColumnSpec, FexuError,
        ModelDescriptor, QueryScope, ReferentialAction,
    };
    use fexu_db::driver::{
        ConnectionTrait, DatabaseConnection, DatabaseTransaction, Statement, Value as DbValue,
    };
    use fexu_db::{DeleteOutcome, ManyToMany};
    use serde_json::{json, Map, Value};
    use std::sync::Arc;
    use uuid::Uuid;

    const ID_FIELD: AdminFieldMeta = AdminFieldMeta {
        name: "id",
        label: "Id",
        list_display: false,
        list_link: false,
        searchable: false,
        sortable: false,
        filterable: false,
        readonly: true,
        hidden: false,
        sensitive: false,
        fieldset: "",
    };
    const TITLE_FIELD: AdminFieldMeta = AdminFieldMeta {
        name: "title",
        label: "Title",
        list_display: true,
        list_link: false,
        searchable: false,
        sortable: false,
        filterable: false,
        readonly: false,
        hidden: false,
        sensitive: false,
        fieldset: "",
    };
    const SHEET_FIELD: AdminFieldMeta = AdminFieldMeta {
        name: "sheet_id",
        label: "Sheet",
        list_display: false,
        list_link: false,
        searchable: false,
        sortable: false,
        filterable: false,
        readonly: false,
        hidden: false,
        sensitive: false,
        fieldset: "",
    };
    const LABEL_FIELD: AdminFieldMeta = AdminFieldMeta {
        name: "label",
        label: "Label",
        list_display: true,
        list_link: false,
        searchable: false,
        sortable: false,
        filterable: false,
        readonly: false,
        hidden: false,
        sensitive: false,
        fieldset: "",
    };
    static SHEET_FIELDS: [AdminFieldMeta; 2] = [ID_FIELD, TITLE_FIELD];
    static LINE_FIELDS: [AdminFieldMeta; 3] = [ID_FIELD, SHEET_FIELD, LABEL_FIELD];

    const fn key_column(name: &'static str) -> ColumnSpec {
        ColumnSpec {
            name,
            kind: ColumnKind::Uuid,
            nullable: false,
            primary_key: true,
            default_bool: None,
            unique: false,
            indexed: false,
            foreign_table: None,
            foreign_column: None,
            on_delete: None,
            precision: 0,
            scale: 0,
            choices: &[],
        }
    }
    const fn text_column(name: &'static str) -> ColumnSpec {
        ColumnSpec {
            name,
            kind: ColumnKind::Text,
            nullable: false,
            primary_key: false,
            default_bool: None,
            unique: false,
            indexed: false,
            foreign_table: None,
            foreign_column: None,
            on_delete: None,
            precision: 0,
            scale: 0,
            choices: &[],
        }
    }
    const SHEET_LINK: ColumnSpec = ColumnSpec {
        name: "sheet_id",
        kind: ColumnKind::Uuid,
        nullable: false,
        primary_key: false,
        default_bool: None,
        unique: false,
        indexed: true,
        foreign_table: Some("probe_sheets"),
        foreign_column: Some("id"),
        on_delete: Some(ReferentialAction::Cascade),
        precision: 0,
        scale: 0,
        choices: &[],
    };
    static SHEET_COLUMNS: [ColumnSpec; 2] = [key_column("id"), text_column("title")];
    static LINE_COLUMNS: [ColumnSpec; 3] = [key_column("id"), SHEET_LINK, text_column("label")];
    static INLINE: [AdminInline; 1] = [AdminInline {
        name: "lines",
        child_table: "probe_lines",
        column: "sheet_id",
        label: "",
        fields: &[],
        readonly_fields: &[],
        min_count: 0,
        max_count: 5,
        extra: 0,
        can_add: true,
        can_change: true,
        can_delete: true,
        ordering: None,
        form: true,
    }];
    static SHEET_META: AdminModelMeta = AdminModelMeta {
        key: "probe_sheets",
        label: "Sheets",
        permission: Some("sheets"),
        fields: &SHEET_FIELDS,
        ordering: None,
        per_page: 25,
        fieldsets: &[],
        inlines: &INLINE,
        date_hierarchy: None,
    };
    static LINE_META: AdminModelMeta = AdminModelMeta {
        key: "probe_lines",
        label: "Lines",
        permission: Some("sheets"),
        fields: &LINE_FIELDS,
        ordering: None,
        per_page: 25,
        fieldsets: &[],
        inlines: &[],
        date_hierarchy: None,
    };
    static SHEET_DESC: ModelDescriptor = ModelDescriptor {
        name: "probe_sheet",
        table: "probe_sheets",
        api_path: "/api/v1/probe_sheets",
        allow_anonymous: false,
        permission: Some("sheets"),
        requires_object_policy: false,
    };
    static LINE_DESC: ModelDescriptor = ModelDescriptor {
        name: "probe_line",
        table: "probe_lines",
        api_path: "/api/v1/probe_lines",
        allow_anonymous: false,
        permission: Some("sheets"),
        requires_object_policy: false,
    };

    struct Sheet;
    struct Line;

    async fn exec(db: &impl ConnectionTrait, sql: &str, values: Vec<DbValue>) {
        let backend = db.get_database_backend();
        db.execute(Statement::from_sql_and_values(backend, sql, values))
            .await
            .unwrap();
    }

    async fn tally(db: &DatabaseConnection, sql: &str) -> usize {
        let backend = db.get_database_backend();
        db.query_all(Statement::from_string(backend, sql.to_owned()))
            .await
            .unwrap()
            .len()
    }

    fn titled(title: &str) -> Map<String, Value> {
        let mut object = Map::new();
        object.insert("title".into(), json!(title));
        object
    }

    async fn title_of(db: &DatabaseConnection, id: Uuid) -> String {
        let backend = db.get_database_backend();
        let row = db
            .query_one(Statement::from_sql_and_values(
                backend,
                "SELECT title FROM probe_sheets WHERE id = ?",
                [DbValue::from(id.to_string())],
            ))
            .await
            .unwrap()
            .unwrap();
        row.try_get("", "title").unwrap()
    }

    #[async_trait::async_trait]
    impl ManagedModel for Sheet {
        fn meta(&self) -> &'static AdminModelMeta {
            &SHEET_META
        }
        fn descriptor(&self) -> &'static ModelDescriptor {
            &SHEET_DESC
        }
        fn columns(&self) -> &'static [ColumnSpec] {
            &SHEET_COLUMNS
        }
        fn policy_bound(&self) -> bool {
            false
        }
        fn read_scope(&self, _: &AuthUser) -> QueryScope {
            QueryScope::All
        }
        fn write_scope(&self, _: &AuthUser) -> QueryScope {
            QueryScope::All
        }
        async fn page(
            &self,
            _: &DatabaseConnection,
            _: &[&str],
            _: Option<&str>,
            _: &str,
            _: bool,
            _: &[(&str, DbValue)],
            _: &[(&str, bool, DbValue)],
            _: u64,
            _: u64,
            _: &QueryScope,
        ) -> Result<(Vec<Value>, bool, i64), FexuError> {
            Ok((Vec::new(), false, 0))
        }
        async fn read(
            &self,
            _: &DatabaseConnection,
            _: Uuid,
            _: &QueryScope,
        ) -> Result<Option<Value>, FexuError> {
            Ok(None)
        }
        async fn create(
            &self,
            _: &DatabaseConnection,
            _: Value,
            _: &AuthUser,
        ) -> Result<Value, FexuError> {
            Err(FexuError::NotFound)
        }
        async fn update(
            &self,
            _: &DatabaseConnection,
            _: Uuid,
            _: Value,
            _: &QueryScope,
            _: &AuthUser,
        ) -> Result<Value, FexuError> {
            Err(FexuError::NotFound)
        }
        async fn remove(
            &self,
            _: &DatabaseConnection,
            _: Uuid,
            _: &QueryScope,
            _: &AuthUser,
        ) -> Result<DeleteOutcome, FexuError> {
            Ok(DeleteOutcome::Missing)
        }
        fn object_allows(&self, _: &AuthUser, _: &str, _: &Value) -> bool {
            true
        }
        async fn read_in(
            &self,
            tx: &DatabaseTransaction,
            id: Uuid,
            _: &QueryScope,
        ) -> Result<Option<Value>, FexuError> {
            let backend = tx.get_database_backend();
            let Some(row) = tx
                .query_one(Statement::from_sql_and_values(
                    backend,
                    "SELECT title FROM probe_sheets WHERE id = ?",
                    [DbValue::from(id.to_string())],
                ))
                .await
                .map_err(FexuError::database)?
            else {
                return Ok(None);
            };
            let title: String = row.try_get("", "title").map_err(FexuError::database)?;
            Ok(Some(json!({ "id": id, "title": title })))
        }
        async fn create_in(
            &self,
            tx: &DatabaseTransaction,
            body: Value,
            _: &AuthUser,
        ) -> Result<Value, FexuError> {
            let id = Uuid::new_v4();
            let title = body["title"].as_str().unwrap_or("").to_owned();
            exec(
                tx,
                "INSERT INTO probe_sheets (id, title) VALUES (?, ?)",
                vec![DbValue::from(id.to_string()), DbValue::from(title.clone())],
            )
            .await;
            Ok(json!({ "id": id.to_string(), "title": title }))
        }
        async fn update_in(
            &self,
            tx: &DatabaseTransaction,
            id: Uuid,
            body: Value,
            _: &QueryScope,
            _: &AuthUser,
        ) -> Result<Value, FexuError> {
            let title = body["title"].as_str().unwrap_or("").to_owned();
            let backend = tx.get_database_backend();
            let result = tx
                .execute(Statement::from_sql_and_values(
                    backend,
                    "UPDATE probe_sheets SET title = ? WHERE id = ?",
                    [DbValue::from(title.clone()), DbValue::from(id.to_string())],
                ))
                .await
                .map_err(FexuError::database)?;
            if result.rows_affected() == 0 {
                return Err(FexuError::NotFound);
            }
            Ok(json!({ "id": id.to_string(), "title": title }))
        }
        async fn delete_in(
            &self,
            _: &DatabaseTransaction,
            _: Uuid,
            _: &QueryScope,
            _: &AuthUser,
        ) -> Result<DeleteOutcome, FexuError> {
            Ok(DeleteOutcome::Missing)
        }
    }

    #[async_trait::async_trait]
    impl ManagedModel for Line {
        fn meta(&self) -> &'static AdminModelMeta {
            &LINE_META
        }
        fn descriptor(&self) -> &'static ModelDescriptor {
            &LINE_DESC
        }
        fn columns(&self) -> &'static [ColumnSpec] {
            &LINE_COLUMNS
        }
        fn policy_bound(&self) -> bool {
            false
        }
        fn read_scope(&self, _: &AuthUser) -> QueryScope {
            QueryScope::All
        }
        fn write_scope(&self, _: &AuthUser) -> QueryScope {
            QueryScope::All
        }
        async fn page(
            &self,
            _: &DatabaseConnection,
            _: &[&str],
            _: Option<&str>,
            _: &str,
            _: bool,
            _: &[(&str, DbValue)],
            _: &[(&str, bool, DbValue)],
            _: u64,
            _: u64,
            _: &QueryScope,
        ) -> Result<(Vec<Value>, bool, i64), FexuError> {
            Ok((Vec::new(), false, 0))
        }
        async fn read(
            &self,
            _: &DatabaseConnection,
            _: Uuid,
            _: &QueryScope,
        ) -> Result<Option<Value>, FexuError> {
            Ok(None)
        }
        async fn create(
            &self,
            _: &DatabaseConnection,
            _: Value,
            _: &AuthUser,
        ) -> Result<Value, FexuError> {
            Err(FexuError::NotFound)
        }
        async fn update(
            &self,
            _: &DatabaseConnection,
            _: Uuid,
            _: Value,
            _: &QueryScope,
            _: &AuthUser,
        ) -> Result<Value, FexuError> {
            Err(FexuError::NotFound)
        }
        async fn remove(
            &self,
            _: &DatabaseConnection,
            _: Uuid,
            _: &QueryScope,
            _: &AuthUser,
        ) -> Result<DeleteOutcome, FexuError> {
            Ok(DeleteOutcome::Missing)
        }
        fn object_allows(&self, _: &AuthUser, _: &str, _: &Value) -> bool {
            true
        }
        async fn read_in(
            &self,
            _: &DatabaseTransaction,
            _: Uuid,
            _: &QueryScope,
        ) -> Result<Option<Value>, FexuError> {
            Ok(None)
        }
        async fn create_in(
            &self,
            tx: &DatabaseTransaction,
            body: Value,
            _: &AuthUser,
        ) -> Result<Value, FexuError> {
            let id = Uuid::new_v4();
            let sheet = body["sheet_id"].as_str().unwrap_or("").to_owned();
            let label = body["label"].as_str().unwrap_or("").to_owned();
            exec(
                tx,
                "INSERT INTO probe_lines (id, sheet_id, label) VALUES (?, ?, ?)",
                vec![
                    DbValue::from(id.to_string()),
                    DbValue::from(sheet),
                    DbValue::from(label),
                ],
            )
            .await;
            Ok(json!({ "id": id.to_string() }))
        }
        async fn update_in(
            &self,
            _: &DatabaseTransaction,
            _: Uuid,
            _: Value,
            _: &QueryScope,
            _: &AuthUser,
        ) -> Result<Value, FexuError> {
            Err(FexuError::NotFound)
        }
        async fn delete_in(
            &self,
            _: &DatabaseTransaction,
            _: Uuid,
            _: &QueryScope,
            _: &AuthUser,
        ) -> Result<DeleteOutcome, FexuError> {
            Ok(DeleteOutcome::Missing)
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn file_failure_rolls_back_record_links_and_inlines() {
        let db = fexu_db::connect("sqlite::memory:").await.unwrap();
        fexu_auth::apply_migrations(&db).await.unwrap();
        let auth = Arc::new(AuthService::new(db.clone(), AuthConfig::development()));
        let created = auth
            .create_user(
                "probe@example.invalid",
                &Uuid::new_v4().simple().to_string(),
                true,
                true,
            )
            .await
            .unwrap();
        let user = AuthUser {
            id: created.id,
            email: created.email,
            is_active: created.is_active,
            is_staff: created.is_staff,
            is_superuser: created.is_superuser,
            session_id: Uuid::new_v4(),
        };
        for sql in [
            "CREATE TABLE probe_sheets (id TEXT PRIMARY KEY, title TEXT NOT NULL)",
            "CREATE TABLE probe_lines (id TEXT PRIMARY KEY, sheet_id TEXT NOT NULL, label TEXT NOT NULL)",
            "CREATE TABLE probe_tags (sheet_id TEXT NOT NULL, tag_id TEXT NOT NULL)",
        ] {
            exec(&db, sql, Vec::new()).await;
        }
        files::ensure(&db).await.unwrap();
        let sheet = Sheet;
        let tag = Uuid::from_u128(7);
        let members = [tag];
        let bytes = b"GIF89a";
        let link = StoredLink {
            name: "tags",
            link: ManyToMany {
                join_table: "probe_tags",
                parent_column: "sheet_id",
                child_column: "tag_id",
            },
            children: &members,
        };
        let file = StoredFile {
            field: "file_name",
            kind: "image/gif",
            bytes,
        };
        let tail = SaveTail {
            links: std::slice::from_ref(&link),
            files: std::slice::from_ref(&file),
            draft: None,
        };
        fail_next_save();
        let failed = commit_model(&db, &user, &sheet, None, titled("Neu"), &tail).await;
        assert!(failed.is_err());
        assert_eq!(tally(&db, "SELECT id FROM probe_sheets").await, 0);
        assert_eq!(tally(&db, "SELECT sheet_id FROM probe_tags").await, 0);

        let existing = Uuid::new_v4();
        exec(
            &db,
            "INSERT INTO probe_sheets (id, title) VALUES (?, ?)",
            vec![DbValue::from(existing.to_string()), DbValue::from("Alt")],
        )
        .await;
        fail_next_save();
        let kept = commit_model(&db, &user, &sheet, Some(existing), titled("Neu"), &tail).await;
        assert!(kept.is_err());
        assert_eq!(title_of(&db, existing).await, "Alt");

        let child: Arc<dyn ManagedModel> = Arc::new(Line);
        let mut parent_body = Map::new();
        parent_body.insert("title".into(), json!("Blatt"));
        let mut form = std::collections::HashMap::new();
        form.insert("inline_lines_0_label".into(), "Zeile".into());
        fail_next_save();
        let inline = save_with(
            &db,
            auth.as_ref(),
            &user,
            &sheet,
            |key| (key == "probe_lines").then(|| child.clone()),
            None,
            parent_body,
            &form,
            SaveTail {
                links: &[],
                files: std::slice::from_ref(&file),
                draft: None,
            },
        )
        .await;
        assert!(matches!(inline, crate::compose::SaveResult::Invalid(_)));
        assert_eq!(tally(&db, "SELECT id FROM probe_sheets").await, 1);
        assert_eq!(tally(&db, "SELECT id FROM probe_lines").await, 0);

        let stored = commit_model(&db, &user, &sheet, None, titled("Bleibt"), &tail)
            .await
            .unwrap();
        assert_eq!(tally(&db, "SELECT id FROM probe_sheets").await, 2);
        assert_eq!(tally(&db, "SELECT sheet_id FROM probe_tags").await, 1);
        let loaded = files::load(&db, "probe_sheets", stored, "file_name")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loaded.1.as_slice(), bytes);
    }
}

#[cfg(test)]
mod password_change_tests {
    use super::posted_password_change;
    use std::collections::HashMap;

    #[test]
    fn missing_password_fields_stay_absent() {
        let mut form = HashMap::new();
        assert!(posted_password_change(&form).is_none());
        form.insert("current_password".into(), String::new());
        form.insert("password".into(), "next-phrase".into());
        form.insert("password_confirm".into(), "next-phrase".into());
        assert!(posted_password_change(&form).is_none());
        form.insert("current_password".into(), "current-phrase".into());
        form.insert("password".into(), String::new());
        assert!(posted_password_change(&form).is_none());
        form.insert("password".into(), "next-phrase".into());
        form.insert("password_confirm".into(), "other-phrase".into());
        assert!(posted_password_change(&form).is_none());
        form.insert("password_confirm".into(), "next-phrase".into());
        assert_eq!(
            posted_password_change(&form),
            Some(("current-phrase", "next-phrase"))
        );
    }
}
