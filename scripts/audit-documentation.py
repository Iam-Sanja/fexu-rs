#!/usr/bin/env python3
"""Static Markdown inventory and structural audit; does not claim API correctness.

Runs without external packages and without network access. A full manual review
against source/tests remains required before publishing versioned documentation.
"""
from __future__ import annotations

import re
import sys
from pathlib import Path
from urllib.parse import unquote, urlparse

ROOT = Path(__file__).resolve().parents[1]
LINK = re.compile(r"!?(?:\[[^\]]*\])\((<?[^\s)>]+>?)")
FENCE = re.compile(r"^\s*(`{3,}|~{3,})")
VERSION = re.compile(r"(?<![\d.])0\.1\.2(?!\d)")
CRITICAL = {
    "README.md",
    "docs/index.md",
    "docs/roadmap.md",
    "docs/quickstart.md",
    "docs/release-candidate-0.2.md",
    "docs/documentation-maintenance.md",
}
# A dated archive may mention a now-closed issue without being wrong.
OUTDATED = (
    "Issue #83 stays open:",
    "resolve PostgreSQL parallel-fixture evidence (#117)",
)
SKIP_SCHEMES = ("http:", "https:", "mailto:", "tel:", "data:", "javascript:")


def markdown_files() -> list[Path]:
    candidates = [ROOT / "README.md", ROOT / "CHANGELOG.md"]
    candidates += sorted((ROOT / "docs").rglob("*.md"))
    candidates += sorted((ROOT / "crates").glob("*/README.md"))
    return [path for path in candidates if path.is_file()]


def main() -> int:
    files = markdown_files()
    problems: list[str] = []
    warnings: list[str] = []
    version_refs: list[str] = []
    links = 0
    for file in files:
        relative = file.relative_to(ROOT).as_posix()
        body = file.read_text(encoding="utf-8")
        fence: tuple[str, int] | None = None
        for number, line in enumerate(body.splitlines(), 1):
            match = FENCE.match(line)
            if match:
                marker = match.group(1)
                if fence is None:
                    fence = (marker[0], len(marker))
                elif marker[0] == fence[0] and len(marker) >= fence[1]:
                    fence = None
                continue
            if fence is not None:
                continue
            if relative in CRITICAL:
                for phrase in OUTDATED:
                    if phrase.casefold() in line.casefold():
                        problems.append(f"{relative}:{number}: stale release blocker: {phrase}")
            if VERSION.search(line):
                version_refs.append(f"{relative}:{number}")
            for target_match in LINK.finditer(line):
                target = unquote(target_match.group(1).strip("<>"))
                if not target or target.startswith(("#", "//", "/")):
                    continue
                if target.lower().startswith(SKIP_SCHEMES):
                    continue
                # URLs with other schemes or Markdown dynamic attributes are
                # not treated as safe local file paths.
                parsed = urlparse(target)
                if parsed.scheme:
                    continue
                path = parsed.path
                if not path or "{" in path or "}" in path:
                    continue
                links += 1
                resolved = (file.parent / path).resolve()
                if not resolved.is_relative_to(ROOT.resolve()):
                    warnings.append(f"{relative}:{number}: link leaves repository: {path}")
                    continue
                if not resolved.exists():
                    problems.append(f"{relative}:{number}: missing local link: {path}")
        if fence is not None:
            problems.append(f"{relative}: unclosed Markdown code fence")

    print(f"Markdown documents examined: {len(files)}")
    print(f"Relative local links examined: {links}")
    print(f"Lines mentioning historical published 0.1.2: {len(version_refs)}")
    print("0.1.2 references are NOT automatically rewritten as 0.2.0.")
    print("Static checks do not validate Rust API claims, examples or external links.")
    for line in warnings[:40]:
        print(f"WARNING: {line}")
    if len(warnings) > 40:
        print(f"... and {len(warnings) - 40} more warnings")
    for line in problems[:60]:
        print(f"ERROR: {line}", file=sys.stderr)
    if len(problems) > 60:
        print(f"... and {len(problems) - 60} more errors", file=sys.stderr)
    if problems:
        print(f"Documentation audit failed with {len(problems)} issue(s).", file=sys.stderr)
        return 1
    print("Static documentation audit passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
