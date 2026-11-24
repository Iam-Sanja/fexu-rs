//! Real Unix worker subprocess recovery against a private SQLite database.
#![cfg(unix)]

use async_trait::async_trait;
use fexu_jobs::{
    apply_migrations, EnqueueOutcome, JobContext, JobError, JobHandler, JobPolicy, JobStatus, Queue,
    SystemClock, When, WorkerConfig, WorkerHost,
};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct WorkerProcess(Child);

impl Drop for WorkerProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Probe {
    root: PathBuf,
}

#[async_trait]
impl JobHandler for Probe {
    const NAME: &'static str = "process_recovery_probe";
    const VERSION: u32 = 1;
    type Payload = serde_json::Value;

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }

    async fn run(&self, _payload: Self::Payload, ctx: &JobContext) -> Result<(), JobError> {
        // Each attempt has its own marker. The parent releases the second
        // attempt only after delivering SIGTERM.
        let marker = self.root.join(format!("{}-{}", ctx.id, ctx.attempt));
        std::fs::write(marker, b"started").expect("write marker");
        while !self.root.join("release").is_file() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Ok(())
    }
}

fn database_url(root: &Path) -> String {
    format!("sqlite://{}?mode=rwc", root.join("jobs.sqlite").display())
}

// Spawn only the current test executable, not a CLI configured by the shell.
fn spawn_worker(root: &Path) -> WorkerProcess {
    WorkerProcess(
        Command::new(std::env::current_exe().expect("test executable"))
            .args(["--exact", "worker_child", "--ignored", "--nocapture"])
            .env("FEXU_TEST_WORKER_ROOT", root)
            .spawn()
            .expect("spawn worker"),
    )
}

#[test]
#[ignore = "subprocess entry point; launched only by worker_process_recovery"]
fn worker_child() {
    let Some(root) = std::env::var_os("FEXU_TEST_WORKER_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    assert!(root.starts_with(std::env::temp_dir()));
    assert!(root
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("fexu-worker-process-")));
    assert!(root.join("jobs.sqlite").is_file());
    let mut host = WorkerHost::new(WorkerConfig {
        database_url: database_url(&root),
        enabled: true,
        poll_ms: 50,
        concurrency: 1,
        policy: JobPolicy {
            lease_secs: 5,
            ..JobPolicy::default()
        },
    });
    host.register(Probe { root }).expect("register");
    host.run_until_ctrl_c().expect("worker");
}

async fn wait_for_marker(child: &mut WorkerProcess, marker: &Path) {
    let deadline = Instant::now() + Duration::from_secs(40);
    loop {
        assert!(child.0.try_wait().expect("child status").is_none());
        if marker.is_file() {
            return;
        }
        assert!(Instant::now() < deadline, "worker did not start attempt");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn enqueue(queue: &Queue) -> Uuid {
    let outcome = queue
        .enqueue_json(
            Probe::NAME,
            1,
            &serde_json::json!({"fixture": true}),
            When {
                max_attempts: Some(2),
                ..When::now()
            },
        )
        .await
        .expect("enqueue");
    let EnqueueOutcome::Created(id) = outcome else {
        panic!("expected new job");
    };
    id
}

#[tokio::test]
async fn worker_process_recovery() {
    let root = std::env::temp_dir().join(format!("fexu-worker-process-{}", Uuid::new_v4()));
    std::fs::create_dir(&root).expect("private fixture directory");
    let scratch = Scratch(root);
    let db = fexu_db::connect(&database_url(&scratch.0))
        .await
        .expect("database");
    apply_migrations(&db).await.expect("migrations");
    let queue = Queue::new(db, Arc::new(SystemClock), JobPolicy::default());
    let id = enqueue(&queue).await;

    let mut first = spawn_worker(&scratch.0);
    wait_for_marker(&mut first, &scratch.0.join(format!("{id}-1"))).await;
    let running = queue.get(id).await.expect("first lease");
    assert_eq!(running.attempts, 1);
    assert_eq!(running.status, JobStatus::Running);
    first.0.kill().expect("kill first worker");
    assert!(!first.0.wait().expect("crashed exit").success());

    // Start before the lease expires; the next process must recover the same
    // persistent job, not create a new one.
    let mut second = spawn_worker(&scratch.0);
    wait_for_marker(&mut second, &scratch.0.join(format!("{id}-2"))).await;
    let recovered = queue.get(id).await.expect("second lease");
    assert_eq!(recovered.attempts, 2);
    assert_eq!(recovered.status, JobStatus::Running);
    assert_ne!(running.lease_token, recovered.lease_token);
    assert_ne!(running.lease_owner, recovered.lease_owner);
    let following = enqueue(&queue).await;

    assert!(Command::new("kill")
        .arg("-TERM")
        .arg(second.0.id().to_string())
        .status()
        .expect("send SIGTERM")
        .success());
    // Leave the handler in flight long enough for the signal task to run.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(second.0.try_wait().expect("draining status").is_none());
    std::fs::write(scratch.0.join("release"), b"finish").expect("release handler");

    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(status) = second.0.try_wait().expect("worker exit") {
            assert!(status.success(), "SIGTERM did not drain the worker");
            break;
        }
        assert!(Instant::now() < deadline, "worker did not stop");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let done = queue.get(id).await.expect("completed recovery");
    assert_eq!(done.status, JobStatus::Succeeded);
    assert_eq!(done.attempts, 2);
    assert_eq!(
        queue.get(following).await.expect("queued").status,
        JobStatus::Pending
    );
    assert!(!scratch.0.join(format!("{following}-1")).exists());
    assert!(!scratch.0.join(format!("{id}-3")).exists());
}
