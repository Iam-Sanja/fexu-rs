//! OpenAPI 3.1 built from registered model metadata.
//!
//! The document lists generated model routes, the health check, the auth routes
//! that `mount` always installs, and operations added with [`super::App::document`].
//! Admin routes are not part of this registry and do not appear. Swagger UI is an
//! optional view of this same document; it is not a second schema generator.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use fexu_auth::{AppState, AuthError};
use fexu_core::{
    problem_json, AdminFieldMeta, AdminModelMeta, ColumnKind, ColumnSpec, FexuModel, FieldContract,
    ModelDescriptor, NumericBound, ReferentialAction,
};
use fexu_db::PAGE_LIMIT;
use serde_json::{json, Map, Value};

use crate::swagger_ui;
use crate::App;

const DOCUMENT_CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'";

const AUTH_PATHS: &[&str] = &[
    "/auth/login",
    "/auth/logout",
    "/auth/me",
    "/auth/csrf",
    "/auth/anonymous",
    "/auth/change-password",
    "/auth/password-reset",
    "/auth/password-reset/confirm",
    "/auth/password-reset/choose",
];

/// Which page, if any, is mounted beside the OpenAPI document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenApiUi {
    Swagger,
    Explorer,
    None,
}

/// Who may read the document and the UI. API routes are unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocsAccess {
    /// Any client that can reach the process.
    Public,
    /// A valid `fexu_session` is required. An anonymous cookie is not enough.
    Session,
}

#[derive(Clone, Debug)]
pub struct OpenApiOptions {
    pub enabled: bool,
    /// Serves the OpenAPI document. The UI cannot be mounted without it.
    pub schema_enabled: bool,
    pub json_path: String,
    pub ui_path: Option<String>,
    pub title: String,
    pub ui: OpenApiUi,
    /// When false, Swagger UI and the explorer do not send API requests.
    pub try_it_out: bool,
    pub access: DocsAccess,
}

impl Default for OpenApiOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            schema_enabled: true,
            json_path: "/api/openapi.json".to_owned(),
            ui_path: Some("/api/docs".to_owned()),
            title: "Fexu".to_owned(),
            ui: OpenApiUi::Swagger,
            try_it_out: false,
            access: DocsAccess::Public,
        }
    }
}

impl OpenApiOptions {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Self::default()
        }
    }

    /// Applies `[api.docs]`. An omitted section is off in production and on otherwise.
    /// An omitted access mode is `session` in production and `public` otherwise.
    pub fn from_project(project: &fexu_migrations::ProjectConfig) -> Self {
        let resolved = project.api_docs.resolve(project.is_production());
        Self::from_resolved(&resolved, project.app_name.clone())
    }

    fn from_resolved(resolved: &fexu_migrations::ResolvedApiDocs, title: String) -> Self {
        let ui = match resolved.ui.as_str() {
            "none" => OpenApiUi::None,
            "explorer" => OpenApiUi::Explorer,
            _ => OpenApiUi::Swagger,
        };
        Self {
            enabled: resolved.enabled,
            schema_enabled: resolved.schema_enabled,
            json_path: resolved.schema_path.clone(),
            ui_path: if ui == OpenApiUi::None {
                None
            } else {
                Some(resolved.path.clone())
            },
            title,
            ui,
            try_it_out: resolved.try_it_out,
            access: if resolved.access == "session" {
                DocsAccess::Session
            } else {
                DocsAccess::Public
            },
        }
    }
}

/// Options for an app that did not call `openapi` or `without_openapi`.
///
/// A loaded project uses `[api.docs]`. Without a project, omitted settings follow
/// `ApiDocsSettings::resolve`: documentation is off in production and on otherwise.
/// [`OpenApiOptions::default`] does not read the environment.
pub(crate) fn implicit_options(project: Option<&fexu_migrations::ProjectConfig>) -> OpenApiOptions {
    if let Some(project) = project {
        return OpenApiOptions::from_project(project);
    }
    let production = matches!(
        fexu_core::aliased_env("FEXU_ENV"),
        Ok(Some(value)) if value.eq_ignore_ascii_case("production")
    );
    if !production {
        return OpenApiOptions::default();
    }
    let resolved = fexu_migrations::ApiDocsSettings::default().resolve(true);
    OpenApiOptions::from_resolved(&resolved, "Fexu".to_owned())
}

/// One extra operation for a route the application already registered.
#[derive(Clone, Debug)]
pub struct OpenApiOperation {
    pub method: String,
    pub path: String,
    pub operation_id: String,
    pub summary: String,
    pub tag: String,
    pub request_schema: Option<Value>,
    pub response_status: u16,
    pub response_schema: Option<Value>,
    /// Session cookie is required.
    pub session: bool,
    /// Cookie writes also require the CSRF header or form field.
    pub csrf: bool,
    pub description: String,
    pub parameters: Vec<Value>,
    pub error_statuses: Vec<u16>,
}

impl OpenApiOperation {
    pub fn new(
        method: impl Into<String>,
        path: impl Into<String>,
        operation_id: impl Into<String>,
    ) -> Self {
        Self {
            method: method.into(),
            path: path.into(),
            operation_id: operation_id.into(),
            summary: String::new(),
            tag: "custom".to_owned(),
            request_schema: None,
            response_status: 200,
            response_schema: None,
            session: false,
            csrf: false,
            description: String::new(),
            parameters: Vec::new(),
            error_statuses: Vec::new(),
        }
    }

    pub fn get(path: impl Into<String>, operation_id: impl Into<String>) -> Self {
        Self::new("GET", path, operation_id)
    }

    pub fn post(path: impl Into<String>, operation_id: impl Into<String>) -> Self {
        Self::new("POST", path, operation_id)
    }

    pub fn summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = summary.into();
        self
    }

    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.tag = tag.into();
        self
    }

    pub fn request_schema(mut self, schema: Value) -> Self {
        self.request_schema = Some(schema);
        self
    }

    pub fn response(mut self, status: u16, schema: Option<Value>) -> Self {
        self.response_status = status;
        self.response_schema = schema;
        self
    }

    pub fn query(mut self, name: &str, schema: Value) -> Self {
        self.parameters.push(json!({
            "name": name,
            "in": "query",
            "required": false,
            "schema": schema
        }));
        self
    }

    pub fn path_parameter(mut self, name: &str, schema: Value) -> Self {
        self.parameters.push(json!({
            "name": name,
            "in": "path",
            "required": true,
            "schema": schema
        }));
        self
    }

    pub fn parameter(mut self, parameter: Value) -> Self {
        self.parameters.push(parameter);
        self
    }

    pub fn error_status(mut self, status: u16) -> Self {
        self.error_statuses.push(status);
        self
    }

    pub fn session(mut self) -> Self {
        self.session = true;
        self
    }

    pub fn csrf(mut self) -> Self {
        self.csrf = true;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ModelAccess {
    Anonymous,
    Permission(&'static str),
    Denied,
    Policy,
    PolicyUnbound,
}

#[derive(Clone, Copy)]
pub(crate) struct DocumentedModel {
    pub descriptor: &'static ModelDescriptor,
    pub columns: &'static [ColumnSpec],
    pub bounds: &'static [NumericBound],
    pub contracts: &'static [FieldContract],
    pub admin: Option<&'static AdminModelMeta>,
    pub access: ModelAccess,
}

impl DocumentedModel {
    pub(crate) fn from_model<M: FexuModel>(access: ModelAccess) -> Self {
        Self {
            descriptor: M::descriptor(),
            columns: M::columns(),
            bounds: M::numeric_bounds(),
            contracts: M::field_contracts(),
            admin: M::admin_meta(),
            access,
        }
    }
}

pub(crate) fn check(app: &App) -> Result<(), String> {
    if !app.openapi.enabled {
        return Ok(());
    }
    let options = &app.openapi;
    if !options.schema_enabled && matches!(options.ui, OpenApiUi::None) {
        return Err(
            "OpenAPI documentation is enabled but neither the schema nor a UI is configured."
                .into(),
        );
    }
    if options.schema_enabled {
        validate_path(&options.json_path)?;
        reject_admin_path(&options.json_path)?;
        if app.paths.contains(&options.json_path) {
            return Err(format!(
                "OpenAPI path `{}` is already registered.",
                options.json_path
            ));
        }
    }
    match options.ui {
        OpenApiUi::None => {
            if options.ui_path.is_some() {
                return Err(
                    "OpenAPI ui_path is set while the UI is none. Clear ui_path or select a UI."
                        .into(),
                );
            }
        }
        OpenApiUi::Swagger | OpenApiUi::Explorer => {
            if !options.schema_enabled {
                return Err(
                    "The API UI needs the OpenAPI schema route. Set schema_enabled or turn the UI off."
                        .into(),
                );
            }
            let Some(ui) = options.ui_path.as_deref() else {
                return Err("The selected API UI needs ui_path.".into());
            };
            validate_path(ui)?;
            reject_admin_path(ui)?;
            if ui == options.json_path {
                return Err("The OpenAPI document and the UI need different paths.".into());
            }
            if app.paths.contains(ui) {
                return Err(format!("OpenAPI UI path `{ui}` is already registered."));
            }
            if options.ui == OpenApiUi::Swagger {
                for suffix in swagger_ui::asset_suffixes() {
                    let asset = format!("{ui}/{suffix}");
                    validate_path(&asset)?;
                    reject_admin_path(&asset)?;
                    if asset == options.json_path || app.paths.contains(&asset) {
                        return Err(format!(
                            "OpenAPI asset path `{asset}` is already registered."
                        ));
                    }
                }
            }
        }
    }
    let mut ids = BTreeSet::new();
    for operation in &app.openapi_operations {
        validate_operation(app, operation, &mut ids)?;
    }
    Ok(())
}

pub(crate) fn specification(app: &App) -> Value {
    let mut paths: BTreeMap<String, Map<String, Value>> = BTreeMap::new();
    let mut schemas: BTreeMap<String, Value> = BTreeMap::new();
    schemas.insert("Problem".to_owned(), problem_schema());
    insert_path(&mut paths, "/healthz", "get", health_operation());
    insert_path(&mut paths, "/readyz", "get", ready_operation());
    for path in AUTH_PATHS {
        for (method, operation) in auth_operations(path) {
            insert_path(&mut paths, path, method, operation);
        }
    }
    let mut models = app.documented.clone();
    models.sort_by_key(|model| model.descriptor.api_path);
    let mut seen_paths = BTreeSet::new();
    for model in models.iter().rev() {
        if !seen_paths.insert(model.descriptor.api_path) {
            continue;
        }
        add_model(app, model, &mut paths, &mut schemas);
    }
    for operation in &app.openapi_operations {
        let method = operation.method.to_ascii_lowercase();
        insert_path(
            &mut paths,
            &operation.path,
            &method,
            custom_operation(operation),
        );
    }
    json!({
        "openapi": "3.1.0",
        "info": {
            "title": app.openapi.title,
            "version": "1",
            "description": "HTTP contract version 1. Each model declares its own path; new projects use /api/v1. POST is not idempotent. PATCH is last-write-wins and does not return 409. Removed operations disappear from this document. Sunset and Deprecation headers are not sent. The crate version stays experimental 0.1.0 until a separate release."
        },
        "paths": path_value(paths),
        "components": {
            "schemas": schema_value(schemas),
            "securitySchemes": security_schemes()
        }
    })
}

pub(crate) fn mount(
    router: Router<AppState>,
    specification: Value,
    options: &OpenApiOptions,
) -> Router<AppState> {
    let spec = Arc::new(specification);
    let options = options.clone();
    let mut router = router;
    if options.schema_enabled {
        let spec = spec.clone();
        let options = options.clone();
        let path = options.json_path.clone();
        router = router.route(
            &path,
            get(move |State(app): State<AppState>, headers: HeaderMap| {
                let spec = spec.clone();
                let options = options.clone();
                async move { serve_json(app, headers, spec, options).await }
            }),
        );
    }
    let Some(ui_path) = options.ui_path.clone() else {
        return router;
    };
    match options.ui {
        OpenApiUi::None => router,
        OpenApiUi::Explorer => {
            let page = explorer_page(&options.title, &options.json_path, options.try_it_out);
            let options = options.clone();
            router.route(
                &ui_path,
                get(move |State(app): State<AppState>, headers: HeaderMap| {
                    let page = page.clone();
                    let options = options.clone();
                    async move { serve_html(app, headers, page, options).await }
                }),
            )
        }
        OpenApiUi::Swagger => mount_swagger(router, options, &ui_path),
    }
}

fn mount_swagger(
    mut router: Router<AppState>,
    options: OpenApiOptions,
    ui_path: &str,
) -> Router<AppState> {
    let page = swagger_ui::page(&options.title, ui_path);
    let script = swagger_ui::initializer(&options.json_path, options.try_it_out);
    let page_options = options.clone();
    router = router.route(
        ui_path,
        get(move |State(app): State<AppState>, headers: HeaderMap| {
            let page = page.clone();
            let options = page_options.clone();
            async move { serve_html(app, headers, page, options).await }
        }),
    );
    let script_options = options.clone();
    let script_path = format!("{ui_path}/swagger-initializer.js");
    router = router.route(
        &script_path,
        get(move |State(app): State<AppState>, headers: HeaderMap| {
            let script = script.clone();
            let options = script_options.clone();
            async move {
                serve_text(
                    app,
                    headers,
                    script,
                    "text/javascript; charset=utf-8",
                    options,
                )
                .await
            }
        }),
    );
    for suffix in swagger_ui::asset_suffixes() {
        let Some((body, content_type)) = swagger_ui::asset(suffix) else {
            continue;
        };
        let path = format!("{ui_path}/{suffix}");
        let options = options.clone();
        router = router.route(
            &path,
            get(move |State(app): State<AppState>, headers: HeaderMap| {
                let options = options.clone();
                async move { serve_bytes(app, headers, body, content_type, options).await }
            }),
        );
    }
    router
}

async fn serve_json(
    app: AppState,
    headers: HeaderMap,
    spec: Arc<Value>,
    options: OpenApiOptions,
) -> Response {
    if let Some(response) = allow_docs(&app, &headers, &options).await {
        return response;
    }
    (
        [(header::CACHE_CONTROL, cache_control(&options, false))],
        Json(spec.as_ref().clone()),
    )
        .into_response()
}

async fn serve_html(
    app: AppState,
    headers: HeaderMap,
    page: String,
    options: OpenApiOptions,
) -> Response {
    if let Some(response) = allow_docs(&app, &headers, &options).await {
        return response;
    }
    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/html; charset=utf-8"),
            ),
            (header::CACHE_CONTROL, cache_control(&options, false)),
            (
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            ),
            (header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY")),
            (
                header::REFERRER_POLICY,
                HeaderValue::from_static("no-referrer"),
            ),
            (
                header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static(DOCUMENT_CSP),
            ),
        ],
        Html(page),
    )
        .into_response()
}

async fn serve_text(
    app: AppState,
    headers: HeaderMap,
    body: String,
    content_type: &'static str,
    options: OpenApiOptions,
) -> Response {
    if let Some(response) = allow_docs(&app, &headers, &options).await {
        return response;
    }
    (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static(content_type)),
            (header::CACHE_CONTROL, cache_control(&options, false)),
            (
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            ),
            (
                header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static("default-src 'none'"),
            ),
        ],
        body,
    )
        .into_response()
}

async fn serve_bytes(
    app: AppState,
    headers: HeaderMap,
    body: &'static [u8],
    content_type: &'static str,
    options: OpenApiOptions,
) -> Response {
    if let Some(response) = allow_docs(&app, &headers, &options).await {
        return response;
    }
    (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static(content_type)),
            (header::CACHE_CONTROL, cache_control(&options, true)),
            (
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            ),
            (
                header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static("default-src 'none'"),
            ),
        ],
        body,
    )
        .into_response()
}

async fn allow_docs(
    app: &AppState,
    headers: &HeaderMap,
    options: &OpenApiOptions,
) -> Option<Response> {
    if options.access != DocsAccess::Session {
        return None;
    }
    let Some(auth) = app.auth.as_ref() else {
        return Some(docs_problem(
            StatusCode::FORBIDDEN,
            "Forbidden",
            "authentication is not configured",
        ));
    };
    match auth.authenticate(headers, &axum::http::Method::GET).await {
        Ok(_) => None,
        Err(AuthError::Forbidden) => Some(docs_problem(
            StatusCode::FORBIDDEN,
            "Forbidden",
            "Access denied",
        )),
        Err(AuthError::OriginRejected) => Some(docs_problem(
            StatusCode::FORBIDDEN,
            "Forbidden",
            "origin is not allowed",
        )),
        Err(_) => Some(docs_problem(
            StatusCode::UNAUTHORIZED,
            "Unauthorized",
            "Authentication required",
        )),
    }
}

fn cache_control(options: &OpenApiOptions, static_asset: bool) -> HeaderValue {
    let value = if options.access == DocsAccess::Session {
        "private, no-store"
    } else if static_asset {
        "public, max-age=86400"
    } else {
        "no-store"
    };
    HeaderValue::from_static(value)
}

fn docs_problem(status: StatusCode, title: &str, detail: &str) -> Response {
    (
        status,
        [
            (header::CACHE_CONTROL, "private, no-store"),
            (header::CONTENT_TYPE, "application/problem+json"),
        ],
        Json(problem_json(status.as_u16(), title, detail)),
    )
        .into_response()
}

fn add_model(
    app: &App,
    model: &DocumentedModel,
    paths: &mut BTreeMap<String, Map<String, Value>>,
    schemas: &mut BTreeMap<String, Value>,
) {
    let name = model.descriptor.name;
    let response = name.to_owned();
    let page = format!("{name}Page");
    let create = format!("{name}Create");
    let update = format!("{name}Update");
    schemas.insert(response.clone(), object_schema(model, SchemaRole::Response));
    schemas.insert(page.clone(), page_schema(&response));
    schemas.insert(create.clone(), object_schema(model, SchemaRole::Create));
    schemas.insert(update.clone(), object_schema(model, SchemaRole::Update));
    let collection = model.descriptor.api_path.trim_end_matches('/');
    let detail = format!("{collection}/{{id}}");
    let tag = name.to_owned();
    match model.access {
        ModelAccess::Denied | ModelAccess::PolicyUnbound => {
            for method in ["get", "post"] {
                insert_path(
                    paths,
                    collection,
                    method,
                    denied_operation(app, &tag, model.access, method, false),
                );
            }
            for method in ["get", "patch", "delete"] {
                insert_path(
                    paths,
                    &detail,
                    method,
                    denied_operation(app, &tag, model.access, method, true),
                );
            }
        }
        access => {
            insert_path(
                paths,
                collection,
                "get",
                list_operation(app, model, &tag, access, &page),
            );
            insert_path(
                paths,
                collection,
                "post",
                write_operation(
                    app,
                    &tag,
                    access,
                    "post",
                    Some(&create),
                    Some(&response),
                    201,
                ),
            );
            insert_path(
                paths,
                &detail,
                "get",
                read_operation(app, &tag, access, "get", Some(&response), 200, true),
            );
            insert_path(
                paths,
                &detail,
                "patch",
                write_operation(
                    app,
                    &tag,
                    access,
                    "patch",
                    Some(&update),
                    Some(&response),
                    200,
                ),
            );
            insert_path(
                paths,
                &detail,
                "delete",
                write_operation(app, &tag, access, "delete", None, None, 204),
            );
        }
    }
}

#[derive(Clone, Copy)]
enum SchemaRole {
    Response,
    Create,
    Update,
}

fn object_schema(model: &DocumentedModel, role: SchemaRole) -> Value {
    let mut properties = Map::new();
    let mut required = Vec::new();
    let mut example = Map::new();
    for column in model.columns {
        let contract = contract_for(model, column.name);
        let include = match role {
            SchemaRole::Response => contract.map(|item| item.api_read).unwrap_or(true),
            SchemaRole::Create => contract
                .map(|item| item.in_create)
                .unwrap_or(!column.primary_key),
            SchemaRole::Update => contract
                .map(|item| item.in_update)
                .unwrap_or(!column.primary_key),
        };
        if !include {
            continue;
        }
        if matches!(role, SchemaRole::Response) && !column.nullable {
            required.push(column.name);
        }
        if matches!(role, SchemaRole::Create)
            && contract
                .map(|item| item.create_required)
                .unwrap_or(!column.nullable && column.default_bool.is_none() && !column.primary_key)
        {
            required.push(column.name);
        }
        properties.insert(
            column.name.to_owned(),
            column_schema(model, column, contract),
        );
        example.insert(
            column.name.to_owned(),
            example_value(model, column, contract),
        );
    }
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": required,
        "properties": Value::Object(properties),
        "example": Value::Object(example)
    })
}

fn column_schema(
    model: &DocumentedModel,
    column: &ColumnSpec,
    contract: Option<&FieldContract>,
) -> Value {
    // Absence on PATCH is `required: false`. Null is accepted only for a nullable column.
    let nullable = column.nullable;
    let mut schema = match column.kind {
        ColumnKind::Uuid => typed("string", nullable, Some("uuid")),
        ColumnKind::Text => typed("string", nullable, None),
        ColumnKind::BigInt => typed("integer", nullable, None),
        ColumnKind::Boolean => typed("boolean", nullable, None),
        ColumnKind::Date => typed("string", nullable, Some("date")),
        ColumnKind::Time => typed("string", nullable, Some("time")),
        ColumnKind::DateTime => typed("string", nullable, Some("date-time")),
        ColumnKind::Decimal => {
            let mut schema = typed("string", nullable, None);
            schema["pattern"] = json!(r"^-?\d+(\.\d+)?$");
            schema["x-fexu-decimal"] = json!({
                "precision": column.precision,
                "scale": column.scale
            });
            schema
        }
        ColumnKind::Json => {
            let mut schema = typed("string", nullable, None);
            schema["contentMediaType"] = json!("application/json");
            schema["description"] = json!(
                "Canonical JSON text. PostgreSQL stores jsonb and SQLite stores text. JSON path operators are not part of this schema."
            );
            schema
        }
        ColumnKind::Choice => {
            let mut values: Vec<Value> = column
                .choices
                .iter()
                .map(|choice| Value::String(choice.value.to_owned()))
                .collect();
            if nullable {
                values.push(Value::Null);
            }
            let mut schema = typed("string", nullable, None);
            schema["enum"] = Value::Array(values);
            schema
        }
    };
    if let Some(contract) = contract {
        if !contract.api_read {
            schema["writeOnly"] = json!(true);
        }
        if let Some(min_length) = contract.min_length {
            schema["minLength"] = json!(min_length);
        }
        if let Some(max_length) = contract.max_length {
            schema["maxLength"] = json!(max_length);
        }
        if contract.format == Some("email") {
            schema["format"] = json!("email");
        } else if contract.format == Some("slug") {
            schema["x-fexu-format"] = json!("slug");
        }
    }
    if let Some(bound) = model
        .bounds
        .iter()
        .find(|bound| bound.column == column.name)
    {
        if column.kind == ColumnKind::BigInt {
            if let Some(min) = bound.min {
                schema["minimum"] = json!(min);
            }
            if let Some(max) = bound.max {
                schema["maximum"] = json!(max);
            }
        } else if column.kind == ColumnKind::Decimal {
            if let Some(min) = bound.min {
                schema["x-fexu-minimum"] = json!(min);
            }
            if let Some(max) = bound.max {
                schema["x-fexu-maximum"] = json!(max);
            }
        }
    }
    if let Some(default) = column.default_bool {
        schema["default"] = json!(default);
    }
    if let (Some(table), Some(referenced)) = (column.foreign_table, column.foreign_column) {
        schema["x-fexu-relation"] = json!({
            "table": table,
            "column": referenced,
            "onDelete": on_delete_name(column.on_delete)
        });
    }
    if let Some(field) = admin_field(model, column.name) {
        if field.filterable {
            schema["x-fexu-filterable"] = json!(true);
        }
        if field.sortable {
            schema["x-fexu-sortable"] = json!(true);
        }
        if field.readonly {
            schema["readOnly"] = json!(
                matches!(
                    contract,
                    Some(item) if !item.in_update
                ) || contract.is_none() && column.primary_key
            );
        }
    }
    schema
}

fn page_schema(item_ref: &str) -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["data", "limit", "next", "has_more"],
        "properties": {
            "data": {
                "type": "array",
                "items": { "$ref": format!("#/components/schemas/{item_ref}") }
            },
            "limit": { "type": "integer", "minimum": 1, "maximum": PAGE_LIMIT },
            "next": { "type": ["string", "null"] },
            "has_more": { "type": "boolean" }
        },
        "description": "Keyset page. Follow next only when has_more is true. The default order is id ascending, and id is always the tie-break. Offset is not accepted. limit defaults to 100 and cannot exceed 100."
    })
}

fn list_operation(
    app: &App,
    model: &DocumentedModel,
    tag: &str,
    access: ModelAccess,
    schema_name: &str,
) -> Value {
    let mut operation = read_operation(app, tag, access, "get", Some(schema_name), 200, false);
    operation["parameters"] = list_parameters(model);
    operation
}

fn list_parameters(model: &DocumentedModel) -> Value {
    let mut sorts = vec!["id".to_owned(), "-id".to_owned()];
    if let Some(admin) = model.admin {
        for field in admin.fields {
            if field.sortable && !field.hidden && field.name != "id" && readable(model, field.name)
            {
                sorts.push(field.name.to_owned());
                sorts.push(format!("-{}", field.name));
            }
        }
    }
    json!([
        {
            "name": "limit",
            "in": "query",
            "required": false,
            "schema": { "type": "integer", "minimum": 1, "maximum": PAGE_LIMIT, "default": PAGE_LIMIT }
        },
        {
            "name": "cursor",
            "in": "query",
            "required": false,
            "schema": { "type": "string" },
            "description": "Opaque keyset token returned as next. A token from another sort is rejected."
        },
        {
            "name": "sort",
            "in": "query",
            "required": false,
            "schema": { "type": "string", "enum": sorts },
            "description": "One allow-listed field. A leading - sorts descending. id is appended when it is not the sort field. Nullable and boolean fields are rejected."
        },
        {
            "name": "filter",
            "in": "query",
            "required": false,
            "schema": { "type": "array", "items": { "type": "string" }, "maxItems": 8 },
            "style": "form",
            "explode": true,
            "description": "Repeat for AND. Forms: field:eq:value, field:ne:value, field:gt:value, field:gte:value, field:lt:value, field:lte:value, field:contains:value, field:isnull, field:notnull. Only readable admin filterable fields. contains is text only."
        }
    ])
}

fn readable(model: &DocumentedModel, name: &str) -> bool {
    model
        .contracts
        .iter()
        .find(|contract| contract.column == name)
        .map(|contract| contract.api_read)
        .unwrap_or(true)
}

fn problem_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["type", "code", "title", "status", "detail", "params"],
        "properties": {
            "type": { "type": "string" },
            "code": { "type": "string" },
            "title": { "type": "string" },
            "status": { "type": "integer" },
            "detail": { "type": "string" },
            "params": { "type": "object", "additionalProperties": { "type": "string" } },
            "field": { "type": "string" }
        }
    })
}

fn read_operation(
    app: &App,
    tag: &str,
    access: ModelAccess,
    method: &str,
    schema_name: Option<&str>,
    status: u16,
    by_id: bool,
) -> Value {
    let mut operation = base_operation(app, tag, method, access, false, by_id);
    if by_id {
        operation["parameters"] = json!([id_parameter()]);
    }
    operation["responses"] = responses(app, access, false, status, schema_name, by_id);
    operation
}

fn write_operation(
    app: &App,
    tag: &str,
    access: ModelAccess,
    method: &str,
    request: Option<&str>,
    response: Option<&str>,
    status: u16,
) -> Value {
    let mut operation = base_operation(app, tag, method, access, true, method != "post");
    if method != "post" {
        operation["parameters"] = json!([id_parameter()]);
    }
    if let Some(request) = request {
        operation["requestBody"] = json!({
            "required": true,
            "content": {
                "application/json": {
                    "schema": { "$ref": format!("#/components/schemas/{request}") }
                }
            }
        });
    }
    operation["responses"] = responses(app, access, true, status, response, method != "post");
    operation
}

fn denied_operation(app: &App, tag: &str, access: ModelAccess, method: &str, by_id: bool) -> Value {
    let mut operation = base_operation(app, tag, method, access, false, by_id);
    if by_id {
        operation["parameters"] = json!([id_parameter()]);
    }
    operation["responses"] = json!({
        "403": problem_response()
    });
    operation
}

fn base_operation(
    app: &App,
    tag: &str,
    method: &str,
    access: ModelAccess,
    write: bool,
    by_id: bool,
) -> Value {
    let operation_id = if by_id {
        format!("{method}_{tag}_by_id")
    } else {
        format!("{method}_{tag}")
    };
    let summary = match access {
        ModelAccess::Anonymous => format!("{method} {tag}"),
        ModelAccess::Permission(name) => {
            format!(
                "{method} {tag}; permission {name}.{}",
                method_action(method)
            )
        }
        ModelAccess::Policy => format!("{method} {tag}; object policy and session"),
        ModelAccess::Denied => format!("{method} {tag}; model is registered without access"),
        ModelAccess::PolicyUnbound => format!("{method} {tag}; object policy is not bound"),
    };
    let mut operation = json!({
        "operationId": operation_id,
        "tags": [tag],
        "summary": summary,
        "security": security_for(app, access, write)
    });
    if write && !matches!(access, ModelAccess::Denied | ModelAccess::PolicyUnbound) {
        operation["description"] = json!(
            "A request that sends fexu_session or fexu_anonymous must also send x-csrf-token or the form field csrf_token. Bearer authorization is not a session."
        );
    }
    operation
}

fn responses(
    app: &App,
    access: ModelAccess,
    write: bool,
    success: u16,
    schema_name: Option<&str>,
    missing_is_404: bool,
) -> Value {
    let mut body = Map::new();
    let success_key = success.to_string();
    body.insert(success_key, success_response(success, schema_name));
    if !matches!(access, ModelAccess::Anonymous) {
        body.insert("401".to_owned(), problem_response());
    }
    body.insert("403".to_owned(), problem_response());
    if missing_is_404 && !matches!(access, ModelAccess::Denied | ModelAccess::PolicyUnbound) {
        body.insert("404".to_owned(), problem_response());
    }
    if write || missing_is_404 {
        body.insert("422".to_owned(), problem_response());
    }
    body.insert("500".to_owned(), problem_response());
    if app.browser_required && write && matches!(access, ModelAccess::Anonymous) {
        body.insert("403".to_owned(), problem_response());
    }
    Value::Object(body)
}

fn success_response(status: u16, schema_name: Option<&str>) -> Value {
    if status == 204 || schema_name.is_none() {
        return json!({ "description": "No response body." });
    }
    json!({
        "description": "JSON document.",
        "content": {
            "application/json": {
                "schema": { "$ref": format!("#/components/schemas/{}", schema_name.unwrap_or_default()) }
            }
        }
    })
}

fn problem_response() -> Value {
    json!({
        "description": "Stable problem JSON. The status, code, title and detail fields are always present.",
        "content": {
            "application/problem+json": {
                "schema": { "$ref": "#/components/schemas/Problem" }
            }
        }
    })
}

fn id_parameter() -> Value {
    json!({
        "name": "id",
        "in": "path",
        "required": true,
        "schema": { "type": "string", "format": "uuid" }
    })
}

fn security_for(app: &App, access: ModelAccess, write: bool) -> Value {
    match access {
        ModelAccess::Anonymous if write && app.browser_required => json!([
            { "sessionCookie": [], "csrfHeader": [] },
            { "anonymousCookie": [], "csrfHeader": [] }
        ]),
        ModelAccess::Anonymous if write => json!([
            {},
            { "sessionCookie": [], "csrfHeader": [] },
            { "anonymousCookie": [], "csrfHeader": [] }
        ]),
        ModelAccess::Anonymous => json!([{}]),
        ModelAccess::Permission(_) | ModelAccess::Policy if write => {
            json!([{ "sessionCookie": [], "csrfHeader": [] }])
        }
        ModelAccess::Permission(_) | ModelAccess::Policy => json!([{ "sessionCookie": [] }]),
        ModelAccess::Denied | ModelAccess::PolicyUnbound => json!([]),
    }
}

fn health_operation() -> Value {
    json!({
        "operationId": "get_healthz",
        "tags": ["health"],
        "summary": "Process is running. This does not check the database.",
        "responses": {
            "200": {
                "description": "Process is running.",
                "content": {
                    "application/json": {
                        "schema": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["status"],
                            "properties": { "status": { "const": "ok" } }
                        }
                    }
                }
            }
        }
    })
}

fn ready_operation() -> Value {
    json!({
        "operationId": "get_readyz",
        "tags": ["health"],
        "summary": "Database accepts SELECT 1. This does not migrate and it does not read migration files.",
        "responses": {
            "200": {
                "description": "Database answered.",
                "content": {
                    "application/json": {
                        "schema": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["status"],
                            "properties": { "status": { "const": "ready" } }
                        }
                    }
                }
            },
            "503": problem_response()
        }
    })
}

fn auth_operations(path: &str) -> Vec<(&'static str, Value)> {
    match path {
        "/auth/login" => vec![(
            "post",
            auth_json("post_auth_login", "Create a session", true, 200, true),
        )],
        "/auth/logout" => vec![(
            "post",
            auth_json("post_auth_logout", "Revoke the session", false, 204, true),
        )],
        "/auth/me" => vec![(
            "get",
            auth_json("get_auth_me", "Read the signed-in user", false, 200, false),
        )],
        "/auth/csrf" => vec![(
            "get",
            auth_json(
                "get_auth_csrf",
                "Read the session CSRF token",
                false,
                200,
                false,
            ),
        )],
        "/auth/anonymous" => vec![(
            "get",
            auth_json(
                "get_auth_anonymous",
                "Open an anonymous session",
                false,
                200,
                false,
            ),
        )],
        "/auth/change-password" => vec![(
            "post",
            auth_json(
                "post_auth_change_password",
                "Replace the password",
                true,
                204,
                true,
            ),
        )],
        "/auth/password-reset" => vec![
            (
                "get",
                auth_html("get_auth_password_reset", "Show the reset request form"),
            ),
            (
                "post",
                auth_html("post_auth_password_reset", "Submit the reset request form"),
            ),
        ],
        "/auth/password-reset/confirm" => {
            vec![(
                "get",
                auth_html("get_auth_password_reset_confirm", "Confirm a reset token"),
            )]
        }
        "/auth/password-reset/choose" => vec![
            (
                "get",
                auth_html(
                    "get_auth_password_reset_choose",
                    "Show the new-password form",
                ),
            ),
            (
                "post",
                auth_html("post_auth_password_reset_choose", "Store the new password"),
            ),
        ],
        _ => Vec::new(),
    }
}

fn auth_json(id: &str, summary: &str, body: bool, status: u16, csrf: bool) -> Value {
    let mut operation = json!({
        "operationId": id,
        "tags": ["auth"],
        "summary": summary,
        "responses": {
            status.to_string(): if status == 204 {
                json!({ "description": "No response body." })
            } else {
                json!({ "description": "JSON document." })
            },
            "401": problem_response(),
            "403": problem_response(),
            "422": problem_response()
        }
    });
    if body {
        operation["requestBody"] = json!({
            "required": true,
            "content": { "application/json": { "schema": { "type": "object" } } }
        });
    }
    if csrf {
        operation["description"] = json!(
            "Cookie sessions need their CSRF proof. HTML admin login uses fexu_login_csrf even when no session cookie is present. Password reset uses fexu_reset_csrf. Neither cookie authorizes another route."
        );
        operation["security"] = json!([
            {},
            { "sessionCookie": [], "csrfHeader": [] },
            { "anonymousCookie": [], "csrfHeader": [] }
        ]);
    }
    operation
}

fn auth_html(id: &str, summary: &str) -> Value {
    json!({
        "operationId": id,
        "tags": ["auth"],
        "summary": summary,
        "responses": {
            "200": {
                "description": "HTML form.",
                "content": { "text/html": { "schema": { "type": "string" } } }
            },
            "403": problem_response()
        }
    })
}

fn custom_operation(operation: &OpenApiOperation) -> Value {
    let mut responses = Map::new();
    responses.insert(
        operation.response_status.to_string(),
        success_response(operation.response_status, None),
    );
    if let Some(schema) = &operation.response_schema {
        responses.insert(
            operation.response_status.to_string(),
            json!({
                "description": "Documented response.",
                "content": { "application/json": { "schema": schema } }
            }),
        );
    }
    for status in &operation.error_statuses {
        responses.insert(status.to_string(), problem_response());
    }
    let mut body = json!({
        "operationId": operation.operation_id,
        "tags": [operation.tag],
        "summary": operation.summary,
        "responses": Value::Object(responses)
    });
    if !operation.description.is_empty() {
        body["description"] = json!(operation.description);
    } else if operation.session || operation.csrf {
        body["description"] = json!(
            "A request that sends fexu_session must also send x-csrf-token when this operation requires that proof. Bearer authorization is not a session."
        );
    }
    if !operation.parameters.is_empty() {
        body["parameters"] = Value::Array(operation.parameters.clone());
    }
    if let Some(schema) = &operation.request_schema {
        body["requestBody"] = json!({
            "required": true,
            "content": { "application/json": { "schema": schema } }
        });
    }
    body["security"] = if operation.session && operation.csrf {
        json!([{ "sessionCookie": [], "csrfHeader": [] }])
    } else if operation.session {
        json!([{ "sessionCookie": [] }])
    } else if operation.csrf {
        json!([{ "csrfHeader": [] }, {}])
    } else {
        json!([{}])
    };
    body
}

fn security_schemes() -> Value {
    json!({
        "sessionCookie": {
            "type": "apiKey",
            "in": "cookie",
            "name": "fexu_session",
            "description": "Signed browser session. It is not a Bearer token."
        },
        "anonymousCookie": {
            "type": "apiKey",
            "in": "cookie",
            "name": "fexu_anonymous",
            "description": "Anonymous browser session. It does not prove an admin login."
        },
        "csrfHeader": {
            "type": "apiKey",
            "in": "header",
            "name": "x-csrf-token",
            "description": "Required for cookie writes. Form posts may send csrf_token instead."
        }
    })
}

fn contract_for<'a>(model: &'a DocumentedModel, column: &str) -> Option<&'a FieldContract> {
    model
        .contracts
        .iter()
        .find(|contract| contract.column == column)
}

fn admin_field<'a>(model: &'a DocumentedModel, column: &str) -> Option<&'a AdminFieldMeta> {
    model
        .admin
        .and_then(|admin| admin.fields.iter().find(|field| field.name == column))
}

fn on_delete_name(action: Option<ReferentialAction>) -> &'static str {
    match action {
        Some(ReferentialAction::Cascade) => "cascade",
        Some(ReferentialAction::SetNull) => "set_null",
        Some(ReferentialAction::Restrict) | None => "restrict",
    }
}

fn example_value(
    model: &DocumentedModel,
    column: &ColumnSpec,
    contract: Option<&FieldContract>,
) -> Value {
    if column.nullable {
        return Value::Null;
    }
    match column.kind {
        ColumnKind::Uuid => json!("00000000-0000-4000-8000-000000000001"),
        ColumnKind::Text => Value::String(example_text(contract)),
        ColumnKind::Json => {
            let text = example_text(contract);
            Value::String(if text.chars().count() <= 2 {
                "{}".to_owned()
            } else {
                text
            })
        }
        ColumnKind::BigInt => {
            let number = model
                .bounds
                .iter()
                .find(|bound| bound.column == column.name)
                .and_then(|bound| bound.min)
                .unwrap_or(1);
            json!(number)
        }
        ColumnKind::Boolean => json!(column.default_bool.unwrap_or(false)),
        ColumnKind::Date => json!("2026-01-02"),
        ColumnKind::Time => json!("12:00:00"),
        ColumnKind::DateTime => json!("2026-01-02T12:00:00Z"),
        ColumnKind::Decimal => {
            if column.scale == 0 {
                json!("1")
            } else {
                json!(format!("1.{}", "0".repeat(column.scale as usize)))
            }
        }
        ColumnKind::Choice => json!(column
            .choices
            .first()
            .map(|choice| choice.value)
            .unwrap_or("")),
    }
}

fn example_text(contract: Option<&FieldContract>) -> String {
    let minimum = contract
        .and_then(|item| item.min_length)
        .unwrap_or(1)
        .max(1) as usize;
    let maximum = contract
        .and_then(|item| item.max_length)
        .map(|value| value as usize)
        .unwrap_or(minimum);
    "a".repeat(minimum.min(maximum).max(1))
}

/// Returns whether `instance` satisfies a schema emitted by this generator.
///
/// Local `#/components/schemas/` references are resolved. The check covers the
/// types, `null`, `required`, `additionalProperties`, `enum`, string length
/// and numeric bounds this document emits.
pub fn matches_schema(document: &Value, schema: &Value, instance: &Value) -> Result<(), String> {
    match_schema(document, schema, instance, 0, "$")
}

fn match_schema(
    document: &Value,
    schema: &Value,
    instance: &Value,
    depth: u8,
    path: &str,
) -> Result<(), String> {
    if depth > 8 {
        return Err(format!("{path} schema reference is too deep"));
    }
    let schema = resolve_schema(document, schema, depth, path)?;
    let object = schema
        .as_object()
        .ok_or_else(|| format!("{path} schema must be an object"))?;
    if let Some(expected) = object.get("const") {
        if instance != expected {
            return Err(format!("{path} must equal {expected}"));
        }
        return Ok(());
    }
    if instance.is_null() {
        let names = type_names(object);
        if names.contains(&"null") {
            return Ok(());
        }
        return Err(format!("{path} is null"));
    }
    if let Some(values) = object.get("enum").and_then(Value::as_array) {
        if !values.iter().any(|value| value == instance) {
            return Err(format!("{path} is outside the enum"));
        }
    }
    let names = type_names(object);
    if names.contains(&"object") || object.contains_key("properties") {
        let value = instance
            .as_object()
            .ok_or_else(|| format!("{path} must be an object"))?;
        let properties = object.get("properties").and_then(Value::as_object);
        let additional = object.get("additionalProperties");
        if additional.and_then(Value::as_bool) == Some(false) {
            let properties =
                properties.ok_or_else(|| format!("{path} object schema has no properties"))?;
            for key in value.keys() {
                if !properties.contains_key(key) {
                    return Err(format!("{path}.{key} is not in the schema"));
                }
            }
        }
        if let Some(required) = object.get("required").and_then(Value::as_array) {
            for name in required {
                let name = name.as_str().unwrap_or("");
                if !value.contains_key(name) {
                    return Err(format!("{path}.{name} is required"));
                }
            }
        }
        if let Some(properties) = properties {
            for (key, property) in properties {
                if let Some(child) = value.get(key) {
                    match_schema(
                        document,
                        property,
                        child,
                        depth + 1,
                        &format!("{path}.{key}"),
                    )?;
                }
            }
        }
        if let Some(extra) = additional.filter(|item| item.is_object()) {
            for (key, child) in value {
                if properties.is_some_and(|map| map.contains_key(key)) {
                    continue;
                }
                match_schema(document, extra, child, depth + 1, &format!("{path}.{key}"))?;
            }
        }
        return Ok(());
    }
    if names.contains(&"array") {
        let items = instance
            .as_array()
            .ok_or_else(|| format!("{path} must be an array"))?;
        let item_schema = object
            .get("items")
            .ok_or_else(|| format!("{path} array schema has no items"))?;
        for (index, item) in items.iter().enumerate() {
            match_schema(
                document,
                item_schema,
                item,
                depth + 1,
                &format!("{path}[{index}]"),
            )?;
        }
        return Ok(());
    }
    if names.contains(&"string") {
        let text = instance
            .as_str()
            .ok_or_else(|| format!("{path} must be a string"))?;
        if object.get("format").and_then(Value::as_str) == Some("uuid")
            && uuid::Uuid::parse_str(text).is_err()
        {
            return Err(format!("{path} must be a uuid"));
        }
        let length = text.chars().count();
        if let Some(min) = object.get("minLength").and_then(Value::as_u64) {
            if length < min as usize {
                return Err(format!("{path} is shorter than minLength"));
            }
        }
        if let Some(max) = object.get("maxLength").and_then(Value::as_u64) {
            if length > max as usize {
                return Err(format!("{path} is longer than maxLength"));
            }
        }
        return Ok(());
    }
    if names.contains(&"integer") {
        let number = instance
            .as_i64()
            .ok_or_else(|| format!("{path} must be an integer"))?;
        if let Some(min) = object.get("minimum").and_then(Value::as_i64) {
            if number < min {
                return Err(format!("{path} is below minimum"));
            }
        }
        if let Some(max) = object.get("maximum").and_then(Value::as_i64) {
            if number > max {
                return Err(format!("{path} is above maximum"));
            }
        }
        return Ok(());
    }
    if names.contains(&"boolean") {
        if !instance.is_boolean() {
            return Err(format!("{path} must be a boolean"));
        }
        return Ok(());
    }
    Err(format!("{path} has no supported schema type"))
}

fn resolve_schema<'a>(
    document: &'a Value,
    schema: &'a Value,
    depth: u8,
    path: &str,
) -> Result<&'a Value, String> {
    let Some(reference) = schema.get("$ref").and_then(Value::as_str) else {
        return Ok(schema);
    };
    let name = reference
        .strip_prefix("#/components/schemas/")
        .ok_or_else(|| format!("{path} reference `{reference}` is not local"))?;
    let next = document
        .pointer(&format!("/components/schemas/{name}"))
        .ok_or_else(|| format!("{path} reference `{reference}` is missing"))?;
    if depth > 8 {
        return Err(format!("{path} schema reference is too deep"));
    }
    Ok(next)
}

fn type_names(schema: &Map<String, Value>) -> Vec<&str> {
    match schema.get("type") {
        Some(Value::String(name)) => vec![name.as_str()],
        Some(Value::Array(items)) => items.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    }
}

fn typed(kind: &str, nullable: bool, format: Option<&str>) -> Value {
    let mut schema = if nullable {
        json!({ "type": [kind, "null"] })
    } else {
        json!({ "type": kind })
    };
    if let Some(format) = format {
        schema["format"] = json!(format);
    }
    schema
}

fn method_action(method: &str) -> &'static str {
    match method {
        "post" => "create",
        "patch" | "put" => "update",
        "delete" => "delete",
        _ => "read",
    }
}

fn insert_path(
    paths: &mut BTreeMap<String, Map<String, Value>>,
    path: &str,
    method: &str,
    operation: Value,
) {
    if path == "/admin" || path.starts_with("/admin/") {
        return;
    }
    let entry = paths.entry(path.to_owned()).or_default();
    entry.insert(method.to_owned(), operation);
}

fn reject_admin_path(path: &str) -> Result<(), String> {
    if path == "/admin" || path.starts_with("/admin/") {
        Err(format!(
            "OpenAPI path `{path}` must not publish an admin route."
        ))
    } else {
        Ok(())
    }
}

fn path_value(paths: BTreeMap<String, Map<String, Value>>) -> Value {
    let mut map = Map::new();
    for (path, methods) in paths {
        map.insert(path, Value::Object(methods));
    }
    Value::Object(map)
}

fn schema_value(schemas: BTreeMap<String, Value>) -> Value {
    let mut map = Map::new();
    for (name, schema) in schemas {
        map.insert(name, schema);
    }
    Value::Object(map)
}

fn validate_operation(
    app: &App,
    operation: &OpenApiOperation,
    ids: &mut BTreeSet<String>,
) -> Result<(), String> {
    let method = operation.method.to_ascii_uppercase();
    if !matches!(method.as_str(), "GET" | "POST" | "PUT" | "PATCH" | "DELETE") {
        return Err(format!(
            "OpenAPI method `{}` is not generated.",
            operation.method
        ));
    }
    validate_path(&operation.path)?;
    if operation.path.starts_with("/admin") {
        return Err("Admin routes are not part of the public OpenAPI document.".into());
    }
    let known = app.paths.contains(&operation.path)
        || operation.path == "/healthz"
        || operation.path == "/readyz"
        || AUTH_PATHS.contains(&operation.path.as_str());
    if !known {
        return Err(format!(
            "OpenAPI path `{}` is not a registered route.",
            operation.path
        ));
    }
    if operation.operation_id.is_empty()
        || !operation
            .operation_id
            .chars()
            .all(|char| char.is_ascii_alphanumeric() || char == '_')
    {
        return Err(format!(
            "OpenAPI operation id `{}` must be an identifier.",
            operation.operation_id
        ));
    }
    if !ids.insert(operation.operation_id.clone()) {
        return Err(format!(
            "OpenAPI operation id `{}` is duplicated.",
            operation.operation_id
        ));
    }
    if !(100..600).contains(&operation.response_status) {
        return Err("OpenAPI response status is outside 100..599.".into());
    }
    for status in &operation.error_statuses {
        if !(100..600).contains(status) {
            return Err("OpenAPI error status is outside 100..599.".into());
        }
        if *status == operation.response_status {
            return Err(format!(
                "OpenAPI operation `{}` repeats status {status}.",
                operation.operation_id
            ));
        }
    }
    for parameter in &operation.parameters {
        let name = parameter.get("name").and_then(Value::as_str).unwrap_or("");
        let location = parameter.get("in").and_then(Value::as_str).unwrap_or("");
        if name.is_empty() || !matches!(location, "query" | "path" | "header" | "cookie") {
            return Err(
                "OpenAPI parameter needs a name and a location of query, path, header or cookie."
                    .into(),
            );
        }
        if parameter.get("schema").is_none() && parameter.get("content").is_none() {
            return Err(format!("OpenAPI parameter `{name}` needs a schema."));
        }
    }
    Ok(())
}

fn validate_path(path: &str) -> Result<(), String> {
    if !path.starts_with('/') || path.contains("//") || path.contains('?') || path.contains('#') {
        return Err(format!(
            "OpenAPI path `{path}` is not a stable absolute path."
        ));
    }
    if path.split('/').skip(1).any(|segment| {
        segment.is_empty()
            || segment.contains("..")
            || segment.contains('\\')
            || segment.contains(' ')
            || !segment.chars().all(|char| {
                char.is_ascii_alphanumeric()
                    || char == '_'
                    || char == '-'
                    || char == '.'
                    || char == '{'
                    || char == '}'
            })
    }) {
        return Err(format!(
            "OpenAPI path `{path}` contains an unsupported segment."
        ));
    }
    Ok(())
}

fn explorer_page(title: &str, json_path: &str, try_it_out: bool) -> String {
    let visible = escape_html(json_path);
    let title = escape_html(title);
    let script_path = serde_json::to_string(json_path).expect("a path string encodes as JSON");
    let form = if try_it_out {
        r##"<h2>Try a request</h2>
<form id="try">
<label>Method <select id="method"><option>GET</option><option>POST</option><option>PUT</option><option>PATCH</option><option>DELETE</option></select></label>
<label>Path <input id="path" size="48" required></label>
<label>JSON body <textarea id="body" rows="4" cols="60"></textarea></label>
<button type="submit">Send</button>
</form>
<pre id="result"></pre>"##
    } else {
        "<p>Try it out is disabled. This page does not send API requests.</p>"
    };
    let script = if try_it_out {
        r##"document.querySelector("#try").addEventListener("submit", async event => {
  event.preventDefault();
  const method = document.querySelector("#method").value;
  const target = pathInput.value;
  const raw = document.querySelector("#body").value.trim();
  const init = { method: method, headers: {}, credentials: "same-origin" };
  if (raw) {
    init.headers["content-type"] = "application/json";
    init.body = raw;
  }
  const response = await fetch(target, init);
  document.querySelector("#result").textContent = response.status + "\n" + await response.text();
});"##
    } else {
        ""
    };
    include_str!("explorer.html")
        .replace("/*TITLE*/", &title)
        .replace("/*SPEC*/", &visible)
        .replace("/*SPEC_JS*/", &script_path)
        .replace("/*TRY_FORM*/", form)
        .replace("/*TRY_JS*/", script)
}

/// Checks the generated document against the OpenAPI 3.1 contract this crate emits.
///
/// The check resolves local schema references, requires operation ids and responses,
/// and rejects a security requirement that names a scheme the document does not define.
pub fn validate_document(document: &Value) -> Result<(), String> {
    let object = document
        .as_object()
        .ok_or("OpenAPI document must be a JSON object")?;
    if object.get("openapi").and_then(Value::as_str) != Some("3.1.0") {
        return Err("openapi must be 3.1.0".into());
    }
    let info = object
        .get("info")
        .and_then(Value::as_object)
        .ok_or("info must be an object")?;
    for key in ["title", "version"] {
        if info
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .is_empty()
        {
            return Err(format!("info.{key} must be a non-empty string"));
        }
    }
    let paths = object
        .get("paths")
        .and_then(Value::as_object)
        .ok_or("paths must be an object")?;
    let schemas = document
        .pointer("/components/schemas")
        .and_then(Value::as_object);
    let schemes = document
        .pointer("/components/securitySchemes")
        .and_then(Value::as_object);
    for (path, item) in paths {
        if path == "/admin" || path.starts_with("/admin/") {
            return Err(format!("admin path `{path}` is in the public document"));
        }
        if !path.starts_with('/') {
            return Err(format!("path `{path}` is not absolute"));
        }
        let methods = item
            .as_object()
            .ok_or_else(|| format!("path `{path}` must be an object"))?;
        for (method, operation_json) in methods {
            if !matches!(
                method.as_str(),
                "get" | "post" | "put" | "patch" | "delete" | "head" | "options" | "trace"
            ) {
                return Err(format!("`{method}` is not an OpenAPI method on `{path}`"));
            }
            let operation = operation_json
                .as_object()
                .ok_or_else(|| format!("`{method} {path}` must be an object"))?;
            if operation
                .get("operationId")
                .and_then(Value::as_str)
                .unwrap_or("")
                .is_empty()
            {
                return Err(format!("`{method} {path}` needs an operationId"));
            }
            let responses = operation
                .get("responses")
                .and_then(Value::as_object)
                .ok_or_else(|| format!("`{method} {path}` needs responses"))?;
            if responses.is_empty() {
                return Err(format!("`{method} {path}` has no responses"));
            }
            for status in responses.keys() {
                if status != "default"
                    && !status
                        .parse::<u16>()
                        .ok()
                        .is_some_and(|code| (100..600).contains(&code))
                {
                    return Err(format!("`{method} {path}` has status `{status}`"));
                }
            }
            if let Some(parameters) = operation.get("parameters") {
                let list = parameters
                    .as_array()
                    .ok_or_else(|| format!("`{method} {path}` parameters must be an array"))?;
                for parameter in list {
                    let name = parameter.get("name").and_then(Value::as_str).unwrap_or("");
                    let location = parameter.get("in").and_then(Value::as_str).unwrap_or("");
                    if name.is_empty()
                        || !matches!(location, "query" | "path" | "header" | "cookie")
                    {
                        return Err(format!(
                            "`{method} {path}` has a parameter without a name or location"
                        ));
                    }
                }
            }
            if let Some(security) = operation.get("security").and_then(Value::as_array) {
                for requirement in security {
                    let requirement = requirement.as_object().ok_or_else(|| {
                        format!("`{method} {path}` security entry must be an object")
                    })?;
                    for name in requirement.keys() {
                        if schemes.is_none_or(|defined| !defined.contains_key(name)) {
                            return Err(format!(
                                "security scheme `{name}` is not defined on `{method} {path}`"
                            ));
                        }
                    }
                }
            }
            check_refs(operation_json, schemas, &format!("{method} {path}"))?;
        }
    }
    if let Some(schemas) = schemas {
        for (name, schema) in schemas {
            if !schema.is_object() {
                return Err(format!("schema `{name}` must be an object"));
            }
            check_refs(schema, Some(schemas), name)?;
        }
    }
    Ok(())
}

fn check_refs(
    value: &Value,
    schemas: Option<&Map<String, Value>>,
    context: &str,
) -> Result<(), String> {
    match value {
        Value::Array(items) => {
            for item in items {
                check_refs(item, schemas, context)?;
            }
        }
        Value::Object(object) => {
            if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
                let name = reference
                    .strip_prefix("#/components/schemas/")
                    .filter(|name| !name.is_empty() && !name.contains('/'))
                    .ok_or_else(|| {
                        format!("`{reference}` in {context} is not a local schema reference")
                    })?;
                if schemas.is_none_or(|defined| !defined.contains_key(name)) {
                    return Err(format!(
                        "schema `{name}` referenced from {context} is missing"
                    ));
                }
            }
            for (key, child) in object {
                if key != "$ref" {
                    check_refs(child, schemas, context)?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn escape_html(value: &str) -> String {
    value
        .chars()
        .map(|char| match char {
            '&' => "&amp;".to_owned(),
            '<' => "&lt;".to_owned(),
            '>' => "&gt;".to_owned(),
            '"' => "&quot;".to_owned(),
            _ => char.to_string(),
        })
        .collect()
}
