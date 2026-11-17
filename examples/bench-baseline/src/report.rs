//! Raw result shape. A missing measurement is `null` or listed in `not_measured`.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Plan {
    pub categories: usize,
    pub items_per_category: usize,
    pub page_limit: u64,
    pub warmup: usize,
    pub sequential_reads: usize,
    pub sequential_writes: usize,
    pub error_each: usize,
    pub concurrent_workers: usize,
    pub concurrent_reads_per_worker: usize,
}

impl Plan {
    pub fn validate(&self) -> Result<(), String> {
        if self.categories == 0 || self.items_per_category == 0 {
            return Err("the fixture needs at least one category and one item".into());
        }
        if self.page_limit == 0 || self.page_limit > 100 {
            return Err("page_limit must be from 1 to 100".into());
        }
        let rows = self.categories.saturating_mul(self.items_per_category);
        if rows as u64 <= self.page_limit {
            return Err("the fixture must be larger than one page so has_more is true".into());
        }
        if self.sequential_reads == 0 || self.sequential_writes == 0 || self.error_each == 0 {
            return Err("each measured phase needs at least one request".into());
        }
        if self.concurrent_workers > 0 && self.concurrent_reads_per_worker == 0 {
            return Err("concurrent workers need at least one read each".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Latency {
    pub min: u64,
    pub p50: u64,
    pub p95: u64,
    pub p99: u64,
    pub max: u64,
}

impl Latency {
    /// Nearest-rank percentile. Index is `ceil(p * n) - 1` on the sorted samples.
    pub fn from_samples(samples: &[u64]) -> Result<Self, String> {
        if samples.is_empty() {
            return Err("latency sample is empty".into());
        }
        let mut sorted = samples.to_vec();
        sorted.sort_unstable();
        Ok(Self {
            min: sorted[0],
            p50: rank(&sorted, 0.50),
            p95: rank(&sorted, 0.95),
            p99: rank(&sorted, 0.99),
            max: sorted[sorted.len() - 1],
        })
    }
}

fn rank(sorted: &[u64], percentile: f64) -> u64 {
    let index = (percentile * sorted.len() as f64).ceil() as usize;
    sorted[index.saturating_sub(1).min(sorted.len() - 1)]
}

#[derive(Debug, Clone, Serialize)]
pub struct Phase {
    pub operation: &'static str,
    pub count: usize,
    pub status: u16,
    pub elapsed_ns: u64,
    pub requests_per_second: f64,
    pub latency_ns: Latency,
}

impl Phase {
    pub fn new(
        operation: &'static str,
        status: u16,
        elapsed_ns: u64,
        samples: &[u64],
    ) -> Result<Self, String> {
        if elapsed_ns == 0 {
            return Err(format!("{operation} elapsed time is zero"));
        }
        let count = samples.len();
        let requests_per_second = count as f64 / (elapsed_ns as f64 / 1_000_000_000.0);
        Ok(Self {
            operation,
            count,
            status,
            elapsed_ns,
            requests_per_second,
            latency_ns: Latency::from_samples(samples)?,
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Memory {
    pub measured: bool,
    pub method: &'static str,
    pub working_set_bytes: Option<u64>,
    pub peak_working_set_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub schema: u32,
    pub framework: &'static str,
    pub crate_version: &'static str,
    pub profile: &'static str,
    pub database: &'static str,
    pub sqlite_pool: &'static str,
    pub transport: &'static str,
    pub plan: Plan,
    pub startup_ready_ns: u64,
    pub auth_setup_ns: u64,
    pub schema_ns: u64,
    pub seed_ns: u64,
    pub login_ns: u64,
    pub login_status: u16,
    pub discover_has_more: bool,
    pub warmup_requests: usize,
    pub sequential_reads: Phase,
    pub sequential_writes: Phase,
    pub unauthorized: Phase,
    pub not_found: Phase,
    pub invalid: Phase,
    pub concurrent_reads: Option<Phase>,
    pub memory: Memory,
    pub not_measured: Vec<&'static str>,
}
