//! Locks the crates.io set. This does not publish.

use std::path::PathBuf;

const PUBLISHABLE: &[&str] = &[
    "fexu",
    "fexu-admin",
    "fexu-api",
    "fexu-auth",
    "fexu-cache",
    "fexu-cli",
    "fexu-core",
    "fexu-db",
    "fexu-forms",
    "fexu-jobs",
    "fexu-macros",
    "fexu-mail",
    "fexu-middleware",
    "fexu-migrations",
    "fexu-static",
    "fexu-storage",
];

const PRIVATE_EXAMPLES: &[&str] = &[
    "admin-browser",
    "bench-baseline",
    "board",
    "contract",
    "desk",
    "myshop",
    "notes",
];

#[test]
fn publishable_crates_share_one_version_and_examples_stay_private() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let workspace = std::fs::read_to_string(root.join("Cargo.toml")).expect("workspace manifest");
    let version = env!("CARGO_PKG_VERSION");
    assert!(workspace.contains(&format!("version = \"{version}\"")));
    for name in PUBLISHABLE {
        let pinned = format!("{name} = {{ path = \"crates/{name}\", version = \"{version}\" }}");
        assert!(
            workspace.contains(&pinned),
            "missing publish pin for {name}"
        );
        let manifest =
            std::fs::read_to_string(root.join(format!("crates/{name}/Cargo.toml"))).unwrap();
        assert!(
            manifest.contains("version.workspace = true"),
            "{name} must use the workspace version"
        );
        assert!(
            !manifest.contains("publish = false"),
            "{name} is one of the sixteen publishable crates"
        );
    }
    for example in PRIVATE_EXAMPLES {
        let manifest =
            std::fs::read_to_string(root.join(format!("examples/{example}/Cargo.toml"))).unwrap();
        assert!(
            manifest.contains("publish = false"),
            "{example} must stay off crates.io"
        );
    }
}
