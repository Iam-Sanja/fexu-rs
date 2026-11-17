//! Small authenticated Fexu application used only as a benchmark fixture.
//!
//! The models call the public `FexuModel` and `App` APIs. This crate does not
//! change framework behavior to improve a number.

mod http;
mod memory;
mod report;

use std::time::Instant;

use fexu::auth::{AuthConfig, AuthService};
use fexu::db::ModelRepository;
use fexu::prelude::*;
use tokio::net::TcpListener;

pub use report::{Latency, Phase, Plan, Report};

const USER_EMAIL: &str = "bench@example.com";

/// 32 hex characters from 16 OS random bytes. The value stays in this process.
fn bench_password() -> String {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).expect("os entropy unavailable");
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Debug, fexu::FexuModel, fexu::serde::Serialize, fexu::serde::Deserialize)]
#[fexu(
    table = "bench_categories",
    api = "/api/v1/bench_categories",
    permissions = "bench",
    admin = true
)]
struct BenchCategory {
    #[field(primary_key, auto)]
    id: Uuid,
    #[field(required, min_length = 1, max_length = 40)]
    #[admin(filterable, sortable, label = "Name")]
    name: String,
}

#[derive(Debug, fexu::FexuModel, fexu::serde::Serialize, fexu::serde::Deserialize)]
#[fexu(
    table = "bench_items",
    api = "/api/v1/bench_items",
    permissions = "bench",
    admin = true
)]
struct BenchItem {
    #[field(primary_key, auto)]
    id: Uuid,
    #[field(required, min_length = 1, max_length = 40)]
    #[admin(filterable, sortable, label = "Name")]
    name: String,
    #[field(foreign_key = "bench_categories.id", on_delete = "restrict")]
    #[admin(filterable, label = "Category")]
    category_id: Uuid,
}

/// Counts for the documented release run.
pub fn release_plan() -> Plan {
    Plan {
        categories: 20,
        items_per_category: 10,
        page_limit: 20,
        warmup: 40,
        sequential_reads: 300,
        sequential_writes: 40,
        error_each: 10,
        concurrent_workers: 4,
        concurrent_reads_per_worker: 40,
    }
}

/// Counts for the correctness test. Still large enough that a page sets `has_more`.
pub fn check_plan() -> Plan {
    Plan {
        categories: 2,
        items_per_category: 3,
        page_limit: 2,
        warmup: 2,
        sequential_reads: 5,
        sequential_writes: 2,
        error_each: 1,
        concurrent_workers: 0,
        concurrent_reads_per_worker: 0,
    }
}

/// Builds the fixture, serves it on loopback, and measures the fixed workload.
pub async fn execute(started: Instant, database_url: &str, plan: &Plan) -> Result<Report, String> {
    plan.validate()?;
    let db = fexu::db::connect(database_url)
        .await
        .map_err(|error| error.to_string())?;
    let auth_started = Instant::now();
    fexu::auth::apply_migrations(&db)
        .await
        .map_err(|error| error.to_string())?;
    let auth = AuthService::new(db.clone(), AuthConfig::development());
    let password = bench_password();
    auth.create_user(USER_EMAIL, &password, false, false)
        .await
        .map_err(|error| error.to_string())?;
    for action in ["read", "create", "update", "delete"] {
        auth.grant_permission("editor", &format!("bench.{action}"))
            .await
            .map_err(|error| error.to_string())?;
    }
    auth.assign_role(USER_EMAIL, "editor")
        .await
        .map_err(|error| error.to_string())?;
    let auth_setup_ns = auth_started.elapsed().as_nanos() as u64;

    let schema_started = Instant::now();
    let app = fexu::App::new(db.clone())
        .with_auth(auth)
        .register::<BenchCategory>()
        .register::<BenchItem>();
    app.bootstrap_schema()
        .await
        .map_err(|error| error.to_string())?;
    let schema_ns = schema_started.elapsed().as_nanos() as u64;

    let seed_started = Instant::now();
    let fixture = seed(&db, plan).await?;
    let seed_ns = seed_started.elapsed().as_nanos() as u64;

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|error| error.to_string())?;
    let address = listener.local_addr().map_err(|error| error.to_string())?;
    let startup_ready_ns = started.elapsed().as_nanos() as u64;
    tokio::spawn(async move {
        let _ = axum::serve(listener, app.into_router()).await;
    });

    let mut client = http::Client::connect(address).await?;
    let login_started = Instant::now();
    let session = client.login(USER_EMAIL, &password).await?;
    let login_ns = login_started.elapsed().as_nanos() as u64;
    drop(password);

    let page = client.discover(&session, plan.page_limit).await?;
    let reads = read_paths(&fixture, &page, plan.page_limit);
    for _ in 0..plan.warmup {
        for path in &reads {
            let status = client.get(&session, path).await?;
            if status != 200 {
                return Err(format!("warmup {path} returned {status}"));
            }
        }
    }

    let sequential_reads = client
        .timed(
            &session,
            plan.sequential_reads,
            "authenticated read",
            |index| reads[index % reads.len()].clone(),
        )
        .await?;
    expect_status(&sequential_reads, 200, "sequential read")?;

    let write_path = format!("/api/v1/bench_items/{}", fixture.item_id);
    let write_body = serde_json::json!({"name": fixture.item_name}).to_string();
    let sequential_writes = client
        .timed_write(
            &session,
            plan.sequential_writes,
            &write_path,
            write_body.as_bytes(),
        )
        .await?;
    expect_status(&sequential_writes, 200, "sequential write")?;

    let unauthorized = client
        .timed_status(
            plan.error_each,
            "/api/v1/bench_items?limit=1",
            None,
            "missing session",
        )
        .await?;
    expect_status(&unauthorized, 401, "missing session")?;
    let missing = format!("/api/v1/bench_items/{}", fixture.missing_id);
    let not_found = client
        .timed_status(plan.error_each, &missing, Some(&session), "missing row")
        .await?;
    expect_status(&not_found, 404, "missing row")?;
    let invalid = client
        .timed_status(
            plan.error_each,
            "/api/v1/bench_items?limit=101",
            Some(&session),
            "rejected page limit",
        )
        .await?;
    expect_status(&invalid, 422, "page limit")?;

    let concurrent_reads = if plan.concurrent_workers == 0 {
        None
    } else {
        Some(
            http::concurrent_reads(
                address,
                &session,
                &reads[0],
                plan.concurrent_workers,
                plan.concurrent_reads_per_worker,
            )
            .await?,
        )
    };

    Ok(Report {
        schema: 1,
        framework: "fexu",
        crate_version: env!("CARGO_PKG_VERSION"),
        profile: if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        database: database_label(database_url),
        sqlite_pool: "max_connections=1 from fexu::db::connect; this bench does not change it",
        transport: "HTTP/1.1 keep-alive on 127.0.0.1, ephemeral port, TCP_NODELAY",
        plan: plan.clone(),
        startup_ready_ns,
        auth_setup_ns,
        schema_ns,
        seed_ns,
        login_ns,
        login_status: 200,
        discover_has_more: page.has_more,
        warmup_requests: plan.warmup * reads.len(),
        sequential_reads,
        sequential_writes,
        unauthorized,
        not_found,
        invalid,
        concurrent_reads,
        memory: memory::sample(),
        not_measured: not_measured(),
    })
}

struct Fixture {
    item_id: Uuid,
    item_name: String,
    category_id: Uuid,
    missing_id: Uuid,
}

struct PageProbe {
    next: String,
    has_more: bool,
}

async fn seed(db: &fexu::db::driver::DatabaseConnection, plan: &Plan) -> Result<Fixture, String> {
    let mut first_category = None;
    let mut first_item = None;
    for category_index in 0..plan.categories {
        let category = BenchCategory::create(
            db,
            BenchCategoryCreate {
                name: format!("cat-{category_index:03}"),
            },
        )
        .await
        .map_err(|error| error.to_string())?;
        if first_category.is_none() {
            first_category = Some(category.id);
        }
        for item_index in 0..plan.items_per_category {
            let name = format!("item-{category_index:03}-{item_index:03}");
            let item = BenchItem::create(
                db,
                BenchItemCreate {
                    name: name.clone(),
                    category_id: category.id,
                },
            )
            .await
            .map_err(|error| error.to_string())?;
            if first_item.is_none() {
                first_item = Some((item.id, name));
            }
        }
    }
    let (item_id, item_name) = first_item.ok_or("seed produced no item")?;
    Ok(Fixture {
        item_id,
        item_name,
        category_id: first_category.ok_or("seed produced no category")?,
        missing_id: Uuid::parse_str("00000000-0000-4000-8000-000000000099")
            .map_err(|error| error.to_string())?,
    })
}

fn read_paths(fixture: &Fixture, page: &PageProbe, limit: u64) -> Vec<String> {
    vec![
        format!("/api/v1/bench_items?limit={limit}&sort=name"),
        format!(
            "/api/v1/bench_items?limit={limit}&cursor={}",
            http::encode_query(&page.next)
        ),
        format!("/api/v1/bench_items/{}", fixture.item_id),
        format!("/api/v1/bench_categories/{}", fixture.category_id),
        format!(
            "/api/v1/bench_items?limit={limit}&filter=category_id:eq:{}",
            fixture.category_id
        ),
    ]
}

fn expect_status(phase: &Phase, status: u16, label: &str) -> Result<(), String> {
    if phase.status == status {
        Ok(())
    } else {
        Err(format!(
            "{label} expected {status}, observed {}",
            phase.status
        ))
    }
}

fn not_measured() -> Vec<&'static str> {
    let mut items = vec!["postgresql", "runique", "cot", "loco", "build_duration"];
    if cfg!(windows) {
        items.extend(["linux", "macos"]);
    } else if cfg!(target_os = "linux") {
        items.extend(["windows", "macos"]);
    } else if cfg!(target_os = "macos") {
        items.extend(["windows", "linux"]);
    }
    items
}

fn database_label(url: &str) -> &'static str {
    if url.starts_with("sqlite:") {
        "sqlite"
    } else {
        "other"
    }
}
