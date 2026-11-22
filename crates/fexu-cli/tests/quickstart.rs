//! A generated project must not embed a machine-local Fexu checkout.

use std::path::Path;
use std::process::Command;

fn fexu(directory: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_fexu"));
    command
        .current_dir(directory)
        .env_remove("FEXU_PASSWORD")
        .env_remove("FEXU_CSRF_SECRET")
        .env_remove("FEXU_MFA_KEY")
        .env_remove("FEXU_SMTP_PASSWORD");
    command
}

fn combined(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn clean_room_project_has_no_local_fexu_path() {
    let parent = std::env::temp_dir().join(format!(
        "fexu-clean-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    std::fs::create_dir_all(&parent).unwrap();
    let created = fexu(&parent)
        .args(["new", "project", "Demo", "--template", "fullstack"])
        .output()
        .unwrap();
    let created_text = combined(&created);
    assert!(created.status.success(), "{created_text}");
    let project = parent.join("Demo");
    let cargo = std::fs::read_to_string(project.join("Cargo.toml")).unwrap();
    assert!(
        cargo.contains("version = \"=0.2.0\""),
        "registry dependency missing: {cargo}"
    );
    assert!(!cargo.contains("path ="), "{cargo}");
    assert!(!cargo.contains("CARGO_MANIFEST_DIR"), "{cargo}");
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap()
        .display()
        .to_string()
        .replace('\\', "/");
    assert!(!cargo.replace('\\', "/").contains(&workspace), "{cargo}");

    // Compile the registry default too: local-checkout generator tests use a different API.
    let mut compile = Command::new("cargo");
    compile
        .current_dir(&project)
        .args(["check", "--all-targets"])
        .env("CARGO_BUILD_JOBS", "2");
    if let Some(target) = std::env::var_os("CARGO_TARGET_DIR") {
        compile.env(
            "CARGO_TARGET_DIR",
            Path::new(&target).join("registry-quickstart"),
        );
    }
    let checked = compile.output().unwrap();
    assert!(checked.status.success(), "{}", combined(&checked));

    let database = project.join("app.sqlite");
    std::fs::File::create(&database).unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        database.to_string_lossy().replace('\\', "/")
    );
    let doctor = fexu(&project)
        .env("DATABASE_URL", &url)
        .arg("doctor")
        .output()
        .unwrap();
    let doctor_text = combined(&doctor);
    assert!(doctor.status.success(), "{doctor_text}");
    assert!(
        doctor_text.contains("SQLite is configured"),
        "{doctor_text}"
    );
    assert!(
        doctor_text.contains("fexu dev does not start a job worker"),
        "{doctor_text}"
    );

    std::fs::remove_dir_all(project.join("templates")).unwrap();
    let missing_templates = fexu(&project)
        .env("DATABASE_URL", &url)
        .arg("doctor")
        .output()
        .unwrap();
    let missing_text = combined(&missing_templates);
    assert!(!missing_templates.status.success(), "{missing_text}");
    assert!(missing_text.contains("templates.missing"), "{missing_text}");
    assert!(
        missing_text.contains("templates.enabled = false"),
        "{missing_text}"
    );
    std::fs::create_dir(project.join("templates")).unwrap();

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let manifest = std::fs::read_to_string(project.join("fexu.toml")).unwrap();
    std::fs::write(
        project.join("fexu.toml"),
        manifest.replace("port = 3000", &format!("port = {port}")),
    )
    .unwrap();
    let busy = fexu(&project)
        .env("DATABASE_URL", &url)
        .arg("doctor")
        .output()
        .unwrap();
    let busy_text = combined(&busy);
    assert!(busy.status.success(), "{busy_text}");
    assert!(busy_text.contains("already in use"), "{busy_text}");
    drop(listener);

    let secret = "not-a-real-csrf-secret-value";
    let invalid = fexu(&project)
        .env("DATABASE_URL", &url)
        .env("FEXU_CSRF_SECRET", secret)
        .env("FEXU_CHECK_SKIP_CARGO", "1")
        .args(["check"])
        .output()
        .unwrap();
    let invalid_text = combined(&invalid);
    assert!(!invalid.status.success(), "{invalid_text}");
    assert!(invalid_text.contains("64 hexadecimal"), "{invalid_text}");
    assert!(!invalid_text.contains(secret), "{invalid_text}");

    let password = "too-short";
    let user = fexu(&project)
        .env("DATABASE_URL", &url)
        .env("FEXU_PASSWORD", password)
        .args([
            "admin",
            "create-user",
            "--email",
            "admin@example.com",
            "--staff",
            "--superuser",
        ])
        .output()
        .unwrap();
    let user_text = combined(&user);
    assert!(!user.status.success(), "{user_text}");
    assert!(user_text.contains("at least 12 characters"), "{user_text}");
    assert!(!user_text.contains(password), "{user_text}");
    assert_eq!(std::fs::metadata(&database).unwrap().len(), 0);

    let toml = std::fs::read_to_string(project.join("fexu.toml")).unwrap();
    std::fs::write(
        project.join("fexu.toml"),
        toml.replace(
            "url = \"sqlite://db.sqlite3?mode=rwc\"",
            "url = \"mysql://ada:s3cret-value@db.internal/app\"",
        ),
    )
    .unwrap();
    let rejected = fexu(&project)
        .env_remove("DATABASE_URL")
        .arg("doctor")
        .output()
        .unwrap();
    let rejected_text = combined(&rejected);
    assert!(!rejected.status.success(), "{rejected_text}");
    assert!(rejected_text.contains("database.url"), "{rejected_text}");
    assert!(!rejected_text.contains("s3cret-value"), "{rejected_text}");

    let _ = std::fs::remove_dir_all(parent);
}
