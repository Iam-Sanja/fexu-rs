use std::ffi::OsString;
use std::process::{Command, Output};

fn fexu() -> Command {
    Command::new(env!("CARGO_BIN_EXE_fexu"))
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn assert_no_panic(output: &Output, label: &str) {
    let rendered = text(output);
    assert_ne!(output.status.code(), Some(101), "{label}\n{rendered}");
    assert!(!rendered.contains("panicked at"), "{label}\n{rendered}");
}

#[test]
fn help_and_subcommand_help_do_not_panic() {
    let output = fexu().arg("--help").output().unwrap();
    assert!(output.status.success(), "{}", text(&output));
    assert!(text(&output).contains("new"), "{}", text(&output));
    for command in [
        "new", "doctor", "check", "upgrade", "dev", "build", "generate", "migrate", "admin",
        "auth", "worker", "jobs", "dumpdata", "loaddata", "manage",
    ] {
        let output = fexu().args([command, "--help"]).output().unwrap();
        assert!(output.status.success(), "{command}\n{}", text(&output));
        assert_no_panic(&output, command);
    }
}

#[test]
fn windows_and_unicode_migration_paths_do_not_panic() {
    let root = std::env::temp_dir().join(format!("fexu-os-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let plain = root.join("migrations");
    let unicode = root.join("pfad-ä-项目");
    std::fs::create_dir_all(&plain).unwrap();
    std::fs::create_dir_all(&unicode).unwrap();

    for (label, path) in [("plain", plain), ("unicode", unicode)] {
        let output = fexu()
            .current_dir(&root)
            .args(["migrate", "status", "--migrations-dir"])
            .arg(&path)
            .output()
            .unwrap();
        assert_no_panic(&output, label);
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
fn unpaired_utf16_surrogate_does_not_panic() {
    use std::os::windows::ffi::OsStringExt;
    let surrogate = OsString::from_wide(&[b'C' as u16, b':' as u16, b'\\' as u16, 0xDCFF]);
    let root = std::env::temp_dir().join(format!("fexu-surrogate-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();

    let unknown = fexu().current_dir(&root).arg(&surrogate).output().unwrap();
    assert_no_panic(&unknown, "unknown command");
    assert!(!unknown.status.success());

    let status = fexu()
        .current_dir(&root)
        .args(["migrate", "status", "--migrations-dir"])
        .arg(&surrogate)
        .output()
        .unwrap();
    assert_no_panic(&status, "migrate status");

    let project = fexu()
        .current_dir(&root)
        .args(["new", "Demo", "--fexu-path"])
        .arg(&surrogate)
        .output()
        .unwrap();
    assert_no_panic(&project, "fexu-path");
    assert!(!project.status.success());
    assert!(!root.join("Demo").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn non_utf8_argument_does_not_panic() {
    use std::os::unix::ffi::OsStringExt;
    let raw = OsString::from_vec(vec![0xff, 0xfe]);
    let output = fexu().arg(&raw).output().unwrap();
    assert_no_panic(&output, "non-utf8 command");
    assert!(!output.status.success());
}
