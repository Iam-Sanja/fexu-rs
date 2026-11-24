//! Diagnostics and checks without modifying the database.

use fexu_migrations::{
    apply_upgrade, discover_project, inspect_directory, load_schema_from_command, plan_upgrade,
    readonly_database_url, render_plan, ProjectConfig, SchemaSnapshot, TEMPLATE_VERSION,
};
use std::process::{Command, ExitCode};

pub fn doctor() -> ExitCode {
    println!("Fexu Doctor\n");
    let mut errors = 0;
    match command_version("rustc") {
        Some(version) => report("OK", &format!("Rust is installed: {version}")),
        None => {
            errors += 1;
            report(
                "ERROR",
                "rustc is missing. Install a stable Rust toolchain.",
            );
        }
    }
    match command_version("cargo") {
        Some(version) => report("OK", &format!("Cargo is installed: {version}")),
        None => {
            errors += 1;
            report("ERROR", "cargo is missing.");
        }
    }
    report("OK", "Docker is optional and not required.");
    let current = std::env::current_dir().ok();
    let Some(current) = current else {
        report("ERROR", "Cannot read the working directory.");
        return ExitCode::from(1);
    };
    match discover_project(&current) {
        Ok(project) => {
            report(
                "OK",
                &format!("fexu.toml found in {}", project.root.display()),
            );
            report(
                "OK",
                "fexu dev does not start a job worker. Run fexu worker separately.",
            );
            if project.migrations_dir.is_dir() {
                report(
                    "OK",
                    &format!("migrations.dir {}", project.migrations_dir.display()),
                );
            } else {
                errors += 1;
                report(
                    "ERROR",
                    "migrations.missing The migrations directory is missing. Fix: create it, or generate the project again with fexu new project. doctor does not migrate.",
                );
            }
            for relative in ["Cargo.toml", "src/main.rs", "src/lib.rs"] {
                let path = project.root.join(relative);
                if path.is_file() {
                    report("OK", &format!("{relative} exists."));
                } else {
                    errors += 1;
                    report(
                        "ERROR",
                        &format!(
                            "project.file.missing {relative} is missing. Fix: run this command inside a directory created by fexu new project."
                        ),
                    );
                }
            }
            if wildcard_interface(&project.host) && !project.allow_wildcard_bind {
                errors += 1;
                report(
                    "ERROR",
                    "server.host is a wildcard interface. Set server.allow_wildcard_bind = true to opt in. The bind address is not a public origin.",
                );
            } else if wildcard_interface(&project.host) {
                report(
                    "OK",
                    "server.host is an explicit wildcard bind. It is not a public origin.",
                );
            } else {
                report(
                    "OK",
                    &format!("Server is bound to {}:{}", project.host, project.port),
                );
                if port_available(&project.host, project.port) {
                    report(
                        "OK",
                        "The configured port accepted a trial bind and was released.",
                    );
                } else {
                    report(
                        "WARN",
                        &format!(
                            "Port {} on {} is already in use. Change server.port or stop the other process.",
                            project.port, project.host
                        ),
                    );
                }
            }
            if project.templates_enabled {
                if project.templates_directory().is_dir() {
                    report("OK", "templates.dir exists.");
                } else {
                    errors += 1;
                    report(
                        "ERROR",
                        &format!(
                            "templates.missing {} is missing. Fix: create that directory or set templates.enabled = false.",
                            project.templates_directory().display()
                        ),
                    );
                }
            }
            match project.database_target() {
                Ok(target) => {
                    if target.source == fexu_migrations::DatabaseUrlSource::Environment {
                        report(
                            "OK",
                            "DATABASE_URL overrides the URL from fexu.toml. Its value is hidden.",
                        );
                    }
                    report(
                        "OK",
                        &format!(
                            "database.target {} ({})",
                            fexu_migrations::redact_database_url(&target.url),
                            target.source
                        ),
                    );
                    if target.url.starts_with("sqlite:") {
                        report(
                            "OK",
                            "SQLite is configured. No database server is required.",
                        );
                    } else if target.url.starts_with("postgres://")
                        || target.url.starts_with("postgresql://")
                    {
                        report(
                            "OK",
                            "PostgreSQL is configured and remains optional. Diagnostics use a read-only connection.",
                        );
                    } else {
                        errors += 1;
                        report(
                            "ERROR",
                            "database.url The database URL must start with sqlite: or postgres:. Fix: set [database] url in fexu.toml or DATABASE_URL. The value is not shown. This command does not open a second resolver.",
                        );
                    }
                }
                Err(error) => {
                    errors += 1;
                    report(
                        "ERROR",
                        &format!(
                            "database.url {} Fix: set [database] url in fexu.toml or DATABASE_URL. The value is not shown. This command does not open a second resolver.",
                            error.0
                        ),
                    );
                }
            }
            let counts = migration_lines(&project, false, false);
            errors += counts.errors;
            let project_counts = project_lines(&project);
            errors += project_counts.errors;
            if project.template.as_deref() == Some("fullstack") || project.embedded_assets {
                report("OK", "The admin UI is embedded in the Rust binary.");
            }
        }
        Err(error) => {
            errors += 1;
            report("ERROR", &error.to_string());
        }
    }
    if errors == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

pub fn check(strict: bool, deploy: bool) -> ExitCode {
    let current = match std::env::current_dir() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(1);
        }
    };
    let project = match discover_project(&current) {
        Ok(project) => project,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(1);
        }
    };
    match fexu_core::aliased_env("FEXU_CSRF_SECRET") {
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(1);
        }
        Ok(Some(value)) => {
            let valid = value.trim().len() == 64
                && value.trim().chars().all(|char| char.is_ascii_hexdigit());
            if !valid {
                eprintln!(
                    "FEXU_CSRF_SECRET must contain 64 hexadecimal characters. Fix: replace it with 64 hex characters. The current value is not shown."
                );
                return ExitCode::from(1);
            }
        }
        Ok(None) => {}
    }
    if let Ok(value) = std::env::var("DATABASE_URL") {
        if value.is_empty() {
            eprintln!("DATABASE_URL is empty.");
            return ExitCode::from(1);
        }
    }
    println!("Fexu project check\n");
    report("OK", "project.config Project configuration is valid.");
    let state = inspect_directory(&project.migrations_dir);
    if !state.conflicts.is_empty() {
        for conflict in state.conflicts {
            eprintln!("migration.order {conflict}");
        }
        return ExitCode::from(1);
    }
    let counts = migration_lines(&project, true, deploy);
    let project_counts = project_lines(&project);
    let warnings = counts.warnings + project_counts.warnings;
    let mut errors = counts.errors + project_counts.errors;
    if deploy {
        let deploy_errors =
            deploy_findings(project.debug, &project.host, project.allow_wildcard_bind);
        errors += deploy_errors;
    }
    if errors > 0 {
        return ExitCode::from(1);
    }
    if std::env::var("FEXU_CHECK_SKIP_CARGO").ok().as_deref() == Some("1") {
        println!("cargo check was skipped for this test run.");
        return if strict && warnings > 0 {
            ExitCode::from(2)
        } else {
            ExitCode::SUCCESS
        };
    }
    println!("Running cargo check. No migrations are applied.");
    let status = Command::new("cargo")
        .arg("check")
        .current_dir(&project.root)
        .status();
    match status {
        Ok(status) if status.success() => {
            if strict && warnings > 0 {
                ExitCode::from(2)
            } else {
                ExitCode::SUCCESS
            }
        }
        Ok(_) => ExitCode::from(1),
        Err(error) => {
            eprintln!("Could not start cargo check: {error}");
            ExitCode::from(1)
        }
    }
}

pub fn upgrade(check_only: bool, dry_run: bool, yes: bool) -> ExitCode {
    let current = match std::env::current_dir() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(1);
        }
    };
    let project = match discover_project(&current) {
        Ok(project) => project,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(1);
        }
    };
    let _ = migration_lines(&project, false, false);
    let plan = match plan_upgrade(&project.root) {
        Ok(plan) => plan,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(1);
        }
    };
    print!(
        "{}",
        render_plan(&crate::text::locale(), &plan, dry_run || check_only)
    );
    println!("No migrations are applied and the database is not reset.");
    let writing = yes && !check_only && !dry_run;
    if !writing {
        println!("No files were written.");
        return if plan.has_conflict() {
            ExitCode::from(1)
        } else {
            ExitCode::SUCCESS
        };
    }
    match apply_upgrade(&project.root, &plan) {
        Ok(true) => {
            println!("Manifest geschrieben: .fexu/project.json");
            println!("Template version for new projects: {TEMPLATE_VERSION}. Existing files remain unchanged.");
            ExitCode::SUCCESS
        }
        Ok(false) => {
            println!("No file changes.");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
    }
}

fn tool(program: &str) -> Command {
    if cfg!(windows) && program == "npm" {
        Command::new("npm.cmd")
    } else {
        Command::new(program)
    }
}

fn report(level: &str, message: &str) {
    println!("[{level}] {}", public_detail(message));
}

/// Hides userinfo in database URLs and `password=` query text.
fn public_detail(text: &str) -> String {
    text.split_inclusive(char::is_whitespace)
        .map(|token| {
            let trimmed = token.trim();
            if trimmed.contains("://") {
                let bare = trimmed.trim_matches(|char| matches!(char, '"' | '\'' | ',' | ')'));
                let redacted = fexu_migrations::redact_database_url(bare);
                token.replacen(trimmed, &redacted, 1)
            } else if trimmed.to_ascii_lowercase().contains("password=") {
                "[redacted] ".to_owned()
            } else {
                token.to_owned()
            }
        })
        .collect()
}

/// Loopback is the only host a diagnostic may probe with a socket.
/// Every other host is reported as available and is not bound.
fn binds_local_port(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "localhost" | "::1")
}

/// Binds a loopback port only for validation. The listener is a temporary and
/// is dropped before this function returns. Non-loopback hosts are not bound.
pub fn port_available(host: &str, port: u16) -> bool {
    port_probe(host, || std::net::TcpListener::bind((host, port)).is_ok())
}

fn port_probe(host: &str, bind: impl FnOnce() -> bool) -> bool {
    if !binds_local_port(host) {
        return true;
    }
    bind()
}

fn wildcard_interface(value: &str) -> bool {
    let value = value.trim();
    value == "0.0.0.0"
        || value == "::"
        || value.starts_with("0.0.0.0:")
        || value.starts_with("[::]:")
}

fn wildcard_bind_opt_in() -> bool {
    matches!(
        std::env::var("FEXU_ALLOW_WILDCARD_BIND").ok().as_deref(),
        Some("1" | "true" | "TRUE")
    )
}

/// Counts distinct canonical IP addresses. Names, empty entries and `*` are ignored.
/// The runtime compares the immediate peer as an IP string; the text is not logged.
pub fn configured_proxy_peers(value: &str) -> usize {
    value
        .split(',')
        .map(str::trim)
        .filter_map(|peer| {
            let ip = peer.parse::<std::net::IpAddr>().ok()?;
            (ip.to_string() == peer).then_some(ip)
        })
        .collect::<std::collections::HashSet<_>>()
        .len()
}

/// Read-only deployment findings. Secret values and peer addresses are not printed.
pub fn deploy_findings(debug: bool, host: &str, file_wildcard_opt_in: bool) -> i32 {
    let mut errors = 0;
    if debug || std::env::var("FEXU_DEBUG").ok().as_deref() == Some("true") {
        errors += 1;
        report(
            "ERROR",
            "deploy.debug Debug is enabled. Turn it off before a deployment.",
        );
    } else {
        report("OK", "deploy.debug Debug is off.");
    }
    let bind = match std::env::var("BIND_ADDRESS") {
        Ok(value) if !value.trim().is_empty() => value,
        _ => host.to_owned(),
    };
    if wildcard_interface(bind.trim()) {
        if file_wildcard_opt_in || wildcard_bind_opt_in() {
            report(
                "OK",
                "deploy.bind Wildcard bind is explicit. It is not added to allowed origins.",
            );
        } else {
            errors += 1;
            report(
                "ERROR",
                "deploy.bind A wildcard bind requires server.allow_wildcard_bind = true or FEXU_ALLOW_WILDCARD_BIND=true. The bind address is not a public origin.",
            );
        }
    } else {
        report(
            "OK",
            "deploy.bind The bind address is not a wildcard interface.",
        );
    }
    let production = std::env::var("FEXU_ENV").ok().as_deref() == Some("production");
    let cookie_secure = match std::env::var("FEXU_COOKIE_SECURE").ok().as_deref() {
        Some("1" | "true" | "TRUE") => true,
        Some(_) => false,
        None => production,
    };
    if cookie_secure {
        report(
            "OK",
            "deploy.cookie Session cookies can use the Secure flag.",
        );
    } else {
        errors += 1;
        report(
            "ERROR",
            "deploy.cookie Set FEXU_COOKIE_SECURE=1 or FEXU_ENV=production.",
        );
    }
    match std::env::var("FEXU_CSRF_SECRET") {
        Ok(value) if csrf_secret_shape_ok(&value) => {
            report(
                "OK",
                "deploy.secret FEXU_CSRF_SECRET is present and has the expected shape. The value is not shown.",
            );
        }
        Ok(_) => {
            errors += 1;
            report(
                "ERROR",
                "deploy.secret FEXU_CSRF_SECRET must contain 64 hexadecimal characters. The value is not shown.",
            );
        }
        Err(_) => {
            errors += 1;
            report(
                "ERROR",
                "deploy.secret FEXU_CSRF_SECRET is missing. A file key is not a deployment secret.",
            );
        }
    }
    match std::env::var("FEXU_MFA_KEY") {
        Ok(value) if csrf_secret_shape_ok(&value) => {
            report(
                "OK",
                "deploy.mfa FEXU_MFA_KEY is present and has the expected shape. The value is not shown.",
            );
        }
        Ok(_) => {
            errors += 1;
            report(
                "ERROR",
                "deploy.mfa FEXU_MFA_KEY must contain 64 hexadecimal characters. The value is not shown.",
            );
        }
        Err(_) => {
            errors += 1;
            report(
                "ERROR",
                "deploy.mfa FEXU_MFA_KEY is missing. A file key is not a deployment secret.",
            );
        }
    }
    if let (Ok(csrf), Ok(mfa)) = (
        std::env::var("FEXU_CSRF_SECRET"),
        std::env::var("FEXU_MFA_KEY"),
    ) {
        if csrf_secret_shape_ok(&csrf)
            && csrf_secret_shape_ok(&mfa)
            && csrf.trim().eq_ignore_ascii_case(mfa.trim())
        {
            errors += 1;
            report(
                "ERROR",
                "deploy.keys CSRF and MFA require independent secrets. Neither value is shown.",
            );
        }
    }
    match std::env::var("FEXU_ALLOWED_ORIGINS") {
        Ok(value) if origins_are_https(&value) => {
            let count = value
                .split(',')
                .map(str::trim)
                .filter(|origin| !origin.is_empty())
                .count();
            report(
                "OK",
                &format!("deploy.origin {count} https origin(s) are configured."),
            );
        }
        Ok(_) => {
            errors += 1;
            report(
                "ERROR",
                "deploy.origin Every FEXU_ALLOWED_ORIGINS entry must be an exact https origin without user info, path, query or fragment.",
            );
        }
        Err(_) => {
            errors += 1;
            report("ERROR", "deploy.origin FEXU_ALLOWED_ORIGINS is missing.");
        }
    }
    let trust = match fexu_core::aliased_env("FEXU_TRUST_PROXY") {
        Ok(Some(value)) => matches!(value.as_str(), "1" | "true" | "TRUE"),
        Ok(None) => false,
        Err(message) => {
            errors += 1;
            report("ERROR", &format!("deploy.proxy {message}"));
            false
        }
    };
    let peers = match fexu_core::aliased_env("FEXU_TRUSTED_PROXIES") {
        Ok(Some(value)) => Some(configured_proxy_peers(&value)),
        Ok(None) => Some(0),
        Err(message) => {
            errors += 1;
            report("ERROR", &format!("deploy.proxy {message}"));
            None
        }
    };
    match (trust, peers) {
        (true, Some(count)) if count > 0 => report(
            "OK",
            &format!(
                "deploy.proxy X-Forwarded-For is accepted from {count} distinct canonical IP peer(s). Names and wildcards are ignored by this check."
            ),
        ),
        (true, Some(_)) => {
            errors += 1;
            report(
                "ERROR",
                "deploy.proxy FEXU_TRUST_PROXY is enabled, but no canonical IP peer is configured. Use the immediate proxy IP, not a name or wildcard.",
            );
        }
        (false, Some(_)) => report(
            "OK",
            "deploy.proxy The client address is the direct peer. X-Forwarded-For is ignored.",
        ),
        (_, None) => {}
    }
    report(
        "INFO",
        "deploy.worker The web process does not start a job worker. Supervise fexu worker separately. This check cannot verify that process is running.",
    );
    report(
        "OK",
        "deploy.shutdown fexu::serve stops on Ctrl+C and, on Unix, on SIGTERM. /healthz does not touch the database. /readyz runs SELECT 1 and does not migrate.",
    );
    errors
}

fn csrf_secret_shape_ok(value: &str) -> bool {
    let value = value.trim();
    value.len() == 64 && value.chars().all(|char| char.is_ascii_hexdigit())
}

fn origins_are_https(value: &str) -> bool {
    let origins: Vec<&str> = value
        .split(',')
        .map(str::trim)
        .filter(|origin| !origin.is_empty())
        .collect();
    !origins.is_empty() && origins.iter().all(|origin| exact_https_origin(origin))
}

// The runtime compares literal browser origins. Accept ASCII DNS names and IP
// literals with an optional numeric port, not arbitrary HTTPS URLs.
fn exact_https_origin(origin: &str) -> bool {
    let Some(authority) = origin.strip_prefix("https://") else {
        return false;
    };
    let (host, port) = if let Some(rest) = authority.strip_prefix('[') {
        let Some((address, suffix)) = rest.split_once(']') else {
            return false;
        };
        if address.parse::<std::net::Ipv6Addr>().is_err() {
            return false;
        }
        let port = if suffix.is_empty() {
            None
        } else if let Some(port) = suffix.strip_prefix(':') {
            Some(port)
        } else {
            return false;
        };
        (None, port)
    } else {
        let (host, port) = match authority.split_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        };
        (Some(host), port)
    };
    if let Some(host) = host {
        if host.is_empty()
            || host.len() > 253
            || !host.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && !label.starts_with('-')
                    && !label.ends_with('-')
                    && label
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            })
        {
            return false;
        }
        if host.bytes().all(|byte| byte.is_ascii_digit() || byte == b'.')
            && host.parse::<std::net::Ipv4Addr>().is_err()
        {
            return false;
        }
    }
    port.is_none_or(|port| {
        !port.is_empty()
            && port.bytes().all(|byte| byte.is_ascii_digit())
            && port.parse::<u16>().is_ok_and(|value| value > 0)
    })
}

fn command_version(program: &str) -> Option<String> {
    let output = tool(program).arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    Some(text.lines().next().unwrap_or(program).trim().to_owned())
}

struct Counts {
    errors: i32,
    warnings: i32,
}

fn migration_lines(project: &ProjectConfig, compare_schema: bool, deploy: bool) -> Counts {
    if !project.apps.is_empty() {
        report("OK", fexu_migrations::runtime_contract_note());
    }
    let url = match project.resolved_database_url() {
        Ok(url) => url,
        Err(error) => {
            report("ERROR", &error.0);
            return Counts {
                errors: 1,
                warnings: 0,
            };
        }
    };
    let readonly = match readonly_database_url(&url) {
        Ok(url) => url,
        Err(error) => {
            report(if deploy { "ERROR" } else { "WARN" }, &error.0);
            return Counts {
                errors: i32::from(deploy),
                warnings: i32::from(!deploy),
            };
        }
    };
    let schema = if compare_schema {
        match load_check_schema(project) {
            Ok(schema) => schema,
            Err(error) => {
                report("ERROR", &format!("migration.schema_conflict {error}"));
                return Counts {
                    errors: 1,
                    warnings: 0,
                };
            }
        }
    } else {
        None
    };
    if compare_schema && schema.is_none() {
        report(
            "OK",
            "migration.schema The model schema was not loaded. fexu migrate status compares it when schema_command or FEXU_SCHEMA_FILE is set.",
        );
    }
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => {
            report("ERROR", &format!("migration.database.unavailable: {error}"));
            return Counts {
                errors: 1,
                warnings: 0,
            };
        }
    };
    let inspected = runtime.block_on(async {
        let db = fexu_db::connect(&readonly)
            .await
            .map_err(|error| format!("migration.database.unavailable: {error}"))?;
        let packages: Vec<fexu_migrations::PackageMigration> = project
            .apps
            .iter()
            .map(|app| fexu_migrations::PackageMigration {
                name: app.name.clone(),
                directory: app.migrations.clone(),
                depends_on: app.depends_on.clone(),
            })
            .collect();
        let auth_files = fexu_auth::embedded_migrations().map_err(|error| error.to_string())?;
        let jobs_files = fexu_jobs::embedded_migrations()
            .map_err(|error| format!("{} {}", error.code(), error.detail()))?;
        fexu_migrations::inspect_framework(
            &db,
            &auth_files,
            &jobs_files,
            fexu_jobs::JOB_HISTORY,
            &project.migrations_dir,
            schema.as_ref(),
            &packages,
        )
        .await
        .map_err(|error| error.0)
    });
    match inspected {
        Ok(report_value) => {
            let mut errors = 0;
            let mut warnings = 0;
            for (scope, status, optional) in [
                ("auth", &report_value.auth, false),
                ("jobs", &report_value.jobs, false),
                ("app", &report_value.app, true),
            ] {
                report("INFO", &migration_scope_readiness(scope, status, optional));
            }
            for package in &report_value.packages {
                report(
                    "INFO",
                    &migration_scope_readiness(&package.name, &package.status, false),
                );
            }
            let jobs_pending = match jobs_pending(&readonly) {
                Ok(pending) => Some(pending),
                Err(error) => {
                    errors += 1;
                    report(
                        "ERROR",
                        &format!("job.migration.unavailable {}", public_detail(&error)),
                    );
                    None
                }
            };
            if report_value.pending_auth() == 0
                && report_value.pending_app() == 0
                && report_value.pending_packages() == 0
                && jobs_pending == Some(false)
                && !report_value.has_error()
            {
                report(
                    "OK",
                    "migration.current Authentication, job, and app migrations are applied.",
                );
            }
            if jobs_pending == Some(true) {
                if deploy {
                    errors += 1;
                    report(
                        "ERROR",
                        "job.migration.pending The job migrations are not fully applied. This check is read-only.",
                    );
                } else {
                    warnings += 1;
                    report(
                        "WARN",
                        "job.migration.pending The job migrations are not fully applied. Run fexu migrate up. This check is read-only.",
                    );
                }
            }
            for item in &report_value.diagnostics {
                let level = match item.level {
                    fexu_migrations::Level::Ok => "OK",
                    fexu_migrations::Level::Warn => "WARN",
                    fexu_migrations::Level::Error => "ERROR",
                };
                report(
                    level,
                    &format!("{} {} {}", item.code, item.migration_id, item.description),
                );
                match item.level {
                    fexu_migrations::Level::Error => errors += 1,
                    fexu_migrations::Level::Warn => warnings += 1,
                    fexu_migrations::Level::Ok => {}
                }
            }
            let pending = report_value.pending_auth()
                + report_value.pending_app()
                + report_value.pending_packages();
            if pending > 0 {
                if deploy {
                    errors += 1;
                    report(
                        "ERROR",
                        "migration.pending Apply fexu migrate up before deployment. This check did not write.",
                    );
                } else {
                    println!("Empfehlung:\n  fexu migrate up");
                }
            }
            Counts { errors, warnings }
        }
        Err(error) => {
            report(
                "ERROR",
                &format!(
                    "{} Fix: check [database] url or DATABASE_URL. This command does not create a database and does not migrate.",
                    public_detail(&error)
                ),
            );
            Counts {
                errors: 1,
                warnings: 0,
            }
        }
    }
}

/// A separate, read-only migration summary for each framework/app component.
fn migration_scope_readiness(
    scope: &str,
    status: &fexu_migrations::StatusReport,
    optional: bool,
) -> String {
    let pending = status
        .lines
        .iter()
        .filter(|line| line.state == "ausstehend")
        .count();
    let state = if !status.conflicts.is_empty() {
        "conflict"
    } else if pending > 0 {
        "pending"
    } else if status.lines.is_empty() {
        if optional {
            "optional-empty"
        } else {
            "missing"
        }
    } else {
        "current"
    };
    format!(
        "migration.readiness.{scope} state={state} pending={pending} conflicts={}",
        status.conflicts.len()
    )
}

fn jobs_pending(url: &str) -> Result<bool, String> {
    let runtime = tokio::runtime::Runtime::new().map_err(|error| error.to_string())?;
    runtime.block_on(async {
        let db = fexu_db::connect(url)
            .await
            .map_err(|error| error.to_string())?;
        let (_text, current) = fexu_jobs::render_migration_status(&db)
            .await
            .map_err(|error| format!("{} {}", error.code(), error.detail()))?;
        Ok(!current)
    })
}

fn load_check_schema(project: &ProjectConfig) -> Result<Option<SchemaSnapshot>, String> {
    if std::env::var("FEXU_CHECK_SKIP_SCHEMA").ok().as_deref() == Some("1") {
        return Ok(None);
    }
    if let Ok(path) = std::env::var("FEXU_SCHEMA_FILE") {
        let text = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
        return Ok(Some(
            SchemaSnapshot::from_json(&text).map_err(|error| error.0)?,
        ));
    }
    let Some(command) = &project.schema_command else {
        return Ok(None);
    };
    Ok(Some(
        load_schema_from_command(&project.root, command).map_err(|error| error.0)?,
    ))
}

struct ProjectCounts {
    errors: i32,
    warnings: i32,
}

fn project_lines(project: &ProjectConfig) -> ProjectCounts {
    let mut warnings = 0;
    let mut errors = 0;
    let cargo = std::fs::read_to_string(project.root.join("Cargo.toml")).unwrap_or_default();
    let dependency = fexu_migrations::fexu_dependency(&cargo);
    match dependency.kind {
        fexu_migrations::DependencyKind::Path => report(
            "OK",
            &format!(
                "project.dependency.path {}",
                dependency.path.as_deref().unwrap_or("")
            ),
        ),
        fexu_migrations::DependencyKind::Git => report("OK", "project.dependency.git"),
        fexu_migrations::DependencyKind::Registry => report(
            "OK",
            &format!(
                "project.dependency.registry Cargo version {} is not the internal template version.",
                crate::scaffold::PUBLISHED_FEXU_VERSION
            ),
        ),
        fexu_migrations::DependencyKind::Missing => {
            report(
                "WARN",
                "project.dependency.unavailable Cargo.toml has no Fexu dependency.",
            );
            warnings += 1;
        }
    }
    if dependency.admin_ui {
        report(
            "OK",
            "project.dependency.admin_ui remains valid for compatibility.",
        );
    }
    if project.static_enabled {
        if project.static_root().is_dir() {
            report("OK", "Static files directory exists.");
        } else {
            report(
                "WARN",
                "project.file.missing The static files directory is missing.",
            );
            warnings += 1;
        }
    }
    match project.enforce_runtime() {
        Ok(()) => report(
            "OK",
            "settings.runtime File settings, defaults, and environment overrides have been merged. No connection is opened.",
        ),
        Err(error) => {
            report("ERROR", &format!("settings.runtime {}", error.0));
            errors += 1;
        }
    }
    match project.mail_backend.as_str() {
        "disabled" => report(
            "OK",
            "mail.disabled No emails are sent and no SMTP server is required.",
        ),
        "console" | "memory" => report(
            "OK",
            "mail.local The backend runs in-process without opening an SMTP connection.",
        ),
        "smtp" => {
            report(
                "OK",
                "mail.smtp The configuration is valid. fexu check does not connect to the server.",
            );
            match fexu_core::aliased_env("FEXU_SMTP_PASSWORD") {
                Err(error) => {
                    report("ERROR", &format!("mail.smtp {error}"));
                    errors += 1;
                }
                Ok(Some(value)) if !value.is_empty() => {}
                Ok(_) => {
                    report(
                        "WARN",
                        "mail.credentials FEXU_SMTP_PASSWORD is missing. Its value is never displayed.",
                    );
                    warnings += 1;
                }
            }
        }
        _ => {
            report("WARN", "mail.backend is not recognized.");
            warnings += 1;
        }
    }
    if project.storage_root().is_dir() {
        report(
            "OK",
            "Storage directory exists. Route registration is not inferred from source code.",
        );
    } else {
        report(
            "WARN",
            "project.file.missing Storage directory is missing. This matters only if the app stores files.",
        );
        warnings += 1;
    }
    for app in &project.apps {
        if app.migrations.is_dir() {
            report(
                "OK",
                &format!(
                    "app.registration `{}` is declared. Rust still registers it with App::install. This command does not load the crate.",
                    app.name
                ),
            );
        } else {
            report(
                "ERROR",
                &format!(
                    "app.registration `{}` declares a missing migrations directory. No database file was created.",
                    app.name
                ),
            );
            errors += 1;
        }
    }
    match plan_upgrade(&project.root) {
        Ok(plan) => {
            for action in &plan.actions {
                if let fexu_migrations::PlanAction::Manual { code, message } = action {
                    if *code == "project.template.legacy" || *code == "project.template.outdated" {
                        report("WARN", &format!("{code} {message}"));
                        warnings += 1;
                    }
                }
                if let fexu_migrations::PlanAction::Conflict {
                    code,
                    relative,
                    message,
                } = action
                {
                    report("WARN", &format!("{code} {relative} {message}"));
                    warnings += 1;
                }
            }
        }
        Err(error) => {
            report("WARN", &error.0);
            warnings += 1;
        }
    }
    ProjectCounts { errors, warnings }
}

#[cfg(test)]
mod tests {
    use super::{
        binds_local_port, configured_proxy_peers, csrf_secret_shape_ok, origins_are_https,
        port_available, port_probe, public_detail,
    };

    #[tokio::test]
    async fn migration_readiness_distinguishes_optional_pending_and_current() {
        let db = fexu_db::connect("sqlite::memory:").await.unwrap();
        let optional = fexu_migrations::StatusReport {
            backend: "SQLite".to_owned(),
            lines: Vec::new(),
            conflicts: Vec::new(),
            schema_note: String::new(),
        };
        assert!(super::migration_scope_readiness("app", &optional, true)
            .contains("state=optional-empty"));
        assert!(
            super::migration_scope_readiness("auth", &optional, false).contains("state=missing")
        );
        let pending = fexu_jobs::migration_status(&db).await.unwrap();
        assert!(super::migration_scope_readiness("jobs", &pending, false)
            .contains("state=pending pending=2 conflicts=0"));
        fexu_jobs::apply_migrations(&db).await.unwrap();
        let current = fexu_jobs::migration_status(&db).await.unwrap();
        assert!(super::migration_scope_readiness("jobs", &current, false)
            .contains("state=current pending=0 conflicts=0"));
    }

    #[test]
    fn jobs_readiness_distinguishes_pending_from_inspection_errors() {
        assert_eq!(super::jobs_pending("sqlite::memory:"), Ok(true));
        assert!(super::jobs_pending("not-a-database-url").is_err());
    }

    #[test]
    fn a_bound_port_is_reported_as_busy_and_released() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(!port_available("127.0.0.1", port));
        // The listener stays open. A non-local host must not bind this port:
        // 0.0.0.0 would conflict with the held socket, and 10.0.0.8 is not local.
        assert!(port_available("10.0.0.8", port));
        assert!(port_available("0.0.0.0", port));
        drop(listener);
    }

    #[test]
    fn only_loopback_hosts_open_a_trial_socket() {
        assert!(binds_local_port("127.0.0.1"));
        assert!(binds_local_port("localhost"));
        assert!(binds_local_port("::1"));
        assert!(!binds_local_port("10.0.0.8"));
        assert!(!binds_local_port("0.0.0.0"));
        let mut called = false;
        assert!(port_probe("10.0.0.8", || {
            called = true;
            false
        }));
        assert!(port_probe("0.0.0.0", || {
            called = true;
            false
        }));
        assert!(!called);
        assert!(!port_probe("localhost", || {
            called = true;
            false
        }));
        assert!(!port_probe("::1", || false));
        assert!(called);
        // A fresh ephemeral port is requested once and dropped inside the probe.
        // The test does not bind that same port again.
        assert!(port_probe("127.0.0.1", || {
            std::net::TcpListener::bind("127.0.0.1:0").is_ok()
        }));
    }

    #[test]
    fn a_proxy_allow_list_ignores_a_wildcard_and_does_not_trust_every_peer() {
        assert_eq!(configured_proxy_peers("10.0.0.1, *"), 1);
        assert_eq!(configured_proxy_peers("*"), 0);
        assert_eq!(configured_proxy_peers(""), 0);
        assert_eq!(configured_proxy_peers(" , "), 0);
        assert_eq!(configured_proxy_peers("proxy, not-an-ip, 999.1.1.1"), 0);
        assert_eq!(configured_proxy_peers("10.0.0.1,10.0.0.1,::1"), 2);
    }

    #[test]
    fn deploy_secret_and_origin_rules_do_not_echo_values() {
        assert!(csrf_secret_shape_ok(&"ab".repeat(32)));
        assert!(!csrf_secret_shape_ok("short"));
        assert!(!csrf_secret_shape_ok(&format!("{}!", "ab".repeat(31))));
        assert!(origins_are_https("https://shop.example"));
        assert!(origins_are_https(
            "https://shop.example, https://admin.example"
        ));
        assert!(!origins_are_https("http://shop.example"));
        assert!(!origins_are_https(""));
        assert!(origins_are_https("https://localhost:8443,https://[::1]:8443"));
        for invalid in [
            "https://",
            "https://*",
            "https://user:password@shop.example",
            "https://shop.example/",
            "https://shop.example/path",
            "https://shop.example?token=secret",
            "https://shop.example#fragment",
            "https://shop.example:0",
            "https://shop.example:65536",
            "https://shop.example:abc",
            "https://999.1.1.1",
            "https://[invalid]",
            "https://shop.example,https://",
        ] {
            assert!(!origins_are_https(invalid));
        }
    }

    #[test]
    fn wildcard_bind_is_an_error_until_it_is_explicit() {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = LOCK.lock().expect("env lock");
        let keys = [
            "BIND_ADDRESS",
            "DATABASE_URL",
            "FEXU_ALLOW_WILDCARD_BIND",
            "FEXU_ALLOWED_ORIGINS",
            "FEXU_COOKIE_SECURE",
            "FEXU_CSRF_SECRET",
            "FEXU_DEBUG",
            "FEXU_ENV",
            "FEXU_MFA_KEY",
            "FEXU_TRUST_PROXY",
            "FEXU_TRUSTED_PROXIES",
        ];
        let saved: Vec<_> = keys
            .iter()
            .map(|key| (*key, std::env::var(key).ok()))
            .collect();
        let secret = "ab".repeat(32);
        std::env::set_var("BIND_ADDRESS", "0.0.0.0:8080");
        std::env::remove_var("DATABASE_URL");
        std::env::remove_var("FEXU_ALLOW_WILDCARD_BIND");
        std::env::set_var("FEXU_ALLOWED_ORIGINS", "https://app.example.test");
        std::env::set_var("FEXU_COOKIE_SECURE", "1");
        std::env::set_var("FEXU_CSRF_SECRET", &secret);
        std::env::set_var("FEXU_MFA_KEY", "cd".repeat(32));
        std::env::remove_var("FEXU_DEBUG");
        std::env::remove_var("FEXU_ENV");
        std::env::remove_var("FEXU_TRUST_PROXY");
        std::env::remove_var("FEXU_TRUSTED_PROXIES");

        assert_eq!(super::deploy_findings(false, "127.0.0.1", false), 1);
        std::env::set_var("FEXU_ALLOW_WILDCARD_BIND", "true");
        assert_eq!(super::deploy_findings(false, "127.0.0.1", false), 0);
        std::env::set_var("BIND_ADDRESS", "[::]:8080");
        std::env::remove_var("FEXU_ALLOW_WILDCARD_BIND");
        assert_eq!(super::deploy_findings(false, "127.0.0.1", false), 1);
        assert_eq!(super::deploy_findings(false, "127.0.0.1", true), 0);
        std::env::set_var("FEXU_MFA_KEY", secret.to_ascii_uppercase());
        assert_eq!(super::deploy_findings(false, "127.0.0.1", true), 1);
        std::env::set_var("FEXU_MFA_KEY", "cd".repeat(32));
        std::env::set_var("FEXU_TRUSTED_PROXIES", "*");
        std::env::set_var("FEXU_TRUST_PROXY", "1");
        assert_eq!(super::deploy_findings(false, "127.0.0.1", true), 1);

        for (key, value) in saved {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }

    #[test]
    fn diagnostic_text_hides_database_credentials() {
        let secret = "postgres://ada:s3cret-value@db.internal:5432/app";
        let rendered = public_detail(&format!("migration.database.unavailable: {secret}"));
        assert!(rendered.contains("postgres://[redacted]@db.internal:5432/app"));
        assert!(!rendered.contains("s3cret-value"));
        assert!(!rendered.contains("ada:"));
        assert_eq!(
            public_detail("sqlite://db.sqlite3?mode=rwc&password=hunter2"),
            "[redacted]"
        );
        assert!(!public_detail("password=hunter2").contains("hunter2"));
    }
}
