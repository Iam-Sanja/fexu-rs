"""Pull request documentation impact gate. Requires only Python stdlib.

Run with PR_BASE_SHA, PR_HEAD_SHA and PR_BODY from the pull_request event.
This makes documentation review explicit; it cannot judge factual correctness.
"""
import os
import re
import subprocess
import sys

PUBLIC_DOCS = "manual/"
SOURCE_ROOTS = (
    "crates/", "examples/", "deploy/", "templates/",
)
SOURCE_FILES = {"Cargo.toml", "Cargo.lock", "compose.yaml"}
SOURCE_SUFFIXES = (".rs", ".toml", ".sql", ".html", ".css", ".js", ".json")
INFRA_ROOTS = (".github/workflows/", "scripts/")


def changed_files(base: str, head: str) -> list[str]:
    result = subprocess.run(
        ["git", "diff", "--name-only", "--diff-filter=ACMRT", base, head],
        capture_output=True, text=True, check=True,
    )
    return [line.strip() for line in result.stdout.splitlines() if line.strip()]


def affects_usage(path: str) -> bool:
    if path in SOURCE_FILES:
        return True
    if path.startswith(SOURCE_ROOTS):
        return path.endswith(SOURCE_SUFFIXES) or path.startswith("deploy/")
    if path.startswith(INFRA_ROOTS):
        return path.endswith((".py", ".sh", ".ps1", ".yml", ".yaml"))
    return False


def docs_exemption(body: str) -> str | None:
    # An explicit, substantive justification; no unreviewed checkbox alone.
    matches = re.findall(
        r"(?im)^Docs-Impact:\s*none\s*[—–-]\s*(\S.{19,})\s*$", body or ""
    )
    return matches[0].strip() if matches else None


def main() -> int:
    base, head = os.environ["PR_BASE_SHA"], os.environ["PR_HEAD_SHA"]
    paths = changed_files(base, head)
    source = sorted(path for path in paths if affects_usage(path))
    manual = sorted(path for path in paths if path.startswith(PUBLIC_DOCS))
    print(f"Changed files: {len(paths)} | user-facing source: {len(source)} | public manual: {len(manual)}")
    if not source:
        print("PASS: no code, CLI, project template, configuration, deployment or workflow changes.")
        return 0
    if manual:
        print("PASS: public manual updated. Reviewers still need to verify the affected pages.")
        for path in manual:
            print(f"  manual: {path}")
        return 0
    exemption = docs_exemption(os.environ.get("PR_BODY", ""))
    if exemption:
        print(f"PASS: PR explicitly records no user-documentation impact: {exemption}")
        return 0
    print("FAIL: changes to user-facing source or tooling need corresponding files in manual/.")
    print("If genuinely no user-visible change, explain why in the PR description:")
    print("Docs-Impact: none — <at least 20 characters of concrete reasoning>")
    for path in source[:25]:
        print(f"  changed: {path}")
    return 1


if __name__ == "__main__":
    sys.exit(main())
