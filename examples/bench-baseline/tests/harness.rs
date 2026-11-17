use std::time::Instant;

#[tokio::test]
async fn check_plan_exercises_auth_crud_relation_page_and_errors() {
    let report = fexu_bench_baseline::execute(
        Instant::now(),
        "sqlite::memory:",
        &fexu_bench_baseline::check_plan(),
    )
    .await
    .expect("baseline fixture");
    assert_eq!(report.framework, "fexu");
    assert_eq!(report.database, "sqlite");
    assert!(report.discover_has_more);
    assert_eq!(report.sequential_reads.status, 200);
    assert_eq!(report.sequential_writes.status, 200);
    assert_eq!(report.unauthorized.status, 401);
    assert_eq!(report.not_found.status, 404);
    assert_eq!(report.invalid.status, 422);
    assert!(report.sequential_reads.requests_per_second > 0.0);
    assert!(report.sequential_reads.latency_ns.p50 <= report.sequential_reads.latency_ns.p95);
    assert!(report.sequential_reads.latency_ns.p95 <= report.sequential_reads.latency_ns.p99);
    assert!(report.concurrent_reads.is_none());
    let json = serde_json::to_string(&report).expect("report json");
    assert!(!json.contains("password"));
    assert!(report.not_measured.contains(&"postgresql"));
    assert!(report.not_measured.contains(&"runique"));
    assert!(report.not_measured.contains(&"cot"));
    assert!(report.not_measured.contains(&"loco"));
    assert!(report.not_measured.contains(&"build_duration"));
    assert!(!report.not_measured.iter().any(|item| {
        (cfg!(windows) && *item == "windows")
            || (cfg!(target_os = "linux") && *item == "linux")
            || (cfg!(target_os = "macos") && *item == "macos")
    }));
}
