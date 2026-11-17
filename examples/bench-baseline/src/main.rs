//! Release entry point for the Fexu baseline. Build time is measured by the script.

use std::path::{Path, PathBuf};
use std::time::Instant;

fn main() {
    let started = Instant::now();
    let outcome = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(run(started));
    if let Err(error) = outcome {
        eprintln!("benchmark failed: {error}");
        std::process::exit(1);
    }
}

async fn run(started: Instant) -> Result<(), String> {
    let mut data = None;
    let mut out = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--data" => data = args.next().map(PathBuf::from),
            "--out" => out = args.next().map(PathBuf::from),
            "--help" | "-h" => {
                eprintln!(
                    "usage: fexu-bench-baseline --data <directory> --out <file.json>\n\nThe directory receives one SQLite file. The JSON file is the raw run."
                );
                return Ok(());
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    let data = data.ok_or("missing --data")?;
    let out = out.ok_or("missing --out")?;
    std::fs::create_dir_all(&data).map_err(|error| error.to_string())?;
    let database = data.join("baseline.sqlite");
    let report = fexu_bench_baseline::execute(
        started,
        &sqlite_url(&database),
        &fexu_bench_baseline::release_plan(),
    )
    .await?;
    if report.profile != "release" {
        return Err(
            "this binary is a debug build; release numbers require cargo build --release".into(),
        );
    }
    let json = serde_json::to_vec_pretty(&report).map_err(|error| error.to_string())?;
    if let Some(parent) = out.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
    }
    std::fs::write(&out, json).map_err(|error| error.to_string())?;
    eprintln!("wrote {}", out.display());
    Ok(())
}

fn sqlite_url(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    format!("sqlite://{text}?mode=rwc")
}
