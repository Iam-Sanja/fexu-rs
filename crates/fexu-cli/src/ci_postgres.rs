//! The PostgreSQL CI job must name real ignored tests.

use std::fs;
use std::path::{Path, PathBuf};

#[test]
fn postgres_ci_names_are_ignored_and_the_job_rejects_a_silent_skip() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let workflow = fs::read_to_string(root.join(".github/workflows/ci.yml")).unwrap();
    let job = postgres_job(&workflow);
    assert!(job.contains("require_passed()"));
    assert!(job.contains("test result: ok\\. [1-9][0-9]* passed"));
    assert!(job.contains("required PostgreSQL command passed no tests"));
    let names = required_names(job);
    assert!(names.len() > 30, "parsed {} names", names.len());
    let sources = rust_sources(&root);
    for name in names {
        let ignored = sources
            .iter()
            .any(|source| function_is_ignored(source, &name));
        assert!(ignored, "{name} is not an ignored test");
    }
}

fn postgres_job(workflow: &str) -> &str {
    let start = workflow
        .find("name: Run the ignored PostgreSQL tests")
        .expect("postgres test step");
    let rest = &workflow[start..];
    let end = rest.find("\n  browser:").expect("browser job");
    &rest[..end]
}

fn required_names(job: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in job.lines() {
        let line = line.trim();
        if !(line.starts_with("require_passed ") || line.starts_with("require_one ")) {
            continue;
        }
        let mut chosen = None;
        for arg in line.split_whitespace() {
            if arg.starts_with('-') {
                continue;
            }
            if arg.contains('_') || arg.contains("::") {
                chosen = Some(arg);
            }
        }
        let raw = chosen.unwrap_or_else(|| panic!("no test filter in {line}"));
        names.push(raw.rsplit("::").next().unwrap().to_owned());
    }
    names
}

fn rust_sources(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    for directory in ["crates", "examples"] {
        collect(root.join(directory), &mut files);
    }
    files
        .into_iter()
        .map(|path| fs::read_to_string(path).unwrap())
        .collect()
}

fn collect(directory: PathBuf, files: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(&directory).unwrap();
    for entry in entries {
        let path = entry.unwrap().path();
        if path.is_dir() {
            if path.file_name().and_then(|name| name.to_str()) == Some("target") {
                continue;
            }
            collect(path, files);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            files.push(path);
        }
    }
}

fn function_is_ignored(source: &str, name: &str) -> bool {
    let needle = format!("fn {name}(");
    let mut rest = source;
    while let Some(index) = rest.find(&needle) {
        let before = &rest[..index];
        let window = before
            .lines()
            .rev()
            .take(12)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
        if window.contains("#[ignore") {
            return true;
        }
        rest = &rest[index + needle.len()..];
    }
    false
}
