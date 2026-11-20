# Fexu documentation audit — 2026-11-20

This record distinguishes **automated checks** from **source-backed editorial review**. It is not a claim that all Rust APIs/examples in the manual have been independently verified, nor that a public documentation website is active.

## Verified source and scope

- Baseline: post-PR #226 development `main` at `e5e140a6c70bbdf9c1f98f4f588119457b5ba910`, with this documentation work in PR #228.
- The GitHub Actions **Documentation build and audit** job initially passed and reported **150 Markdown documents**, **845 relative local links** and **87 lines containing historical published `0.1.2` references**. The auditor checks path existence, code fences and a small set of known misleading release claims.
- `mkdocs build` produced a static HTML site from `docs/`. Its first run showed warnings because Markdown pages linked to repository-only documents such as `../README.md`, `../SECURITY.md` and workflow files. PR #228 replaces these with explicit GitHub source links. Re-run the job on the latest PR head to verify the warning fix.
- These figures are from a CI run and may change with subsequent edits. A static link existing does not prove that a Markdown heading anchor, external URL or code example is correct.

## Corrected in this documentation PR

| Area | Change |
|---|---|
| `README.md` | Explicitly marks **all 16** Fexu crates as targeting coordinated `0.2.0`; crates.io badges and published `0.1.2` install examples remain truthful |
| `docs/roadmap.md` | Updated review baseline to after merged #226; removed obsolete open blocker claims for completed #83 and #117, retained live #92/#93 gates |
| `mkdocs.yml` / `requirements-docs.txt` | Added navigable, searchable MkDocs Material site generated from existing Markdown sources; `mike` is available for per-release frozen snapshots |
| `.github/workflows/docs-check.yml` | Tests documentation on PRs with **read-only** GitHub permissions; never deploys Pages or changes release aliases |
| `scripts/audit-documentation.py` | Reproducible offline structural scan; refuses certain critically outdated claims |
| `docs/documentation-maintenance.md` | Editorial policy and release-only docs versioning/rollback procedure |
| Source-only Markdown links | Repointed links outside `docs/` to their GitHub repository paths for browser usability |

## Not yet proven: required review before 0.2.0

1. Verify each user-facing CLI and installation snippet against the **actual versioned candidate**, not only `main`. The current `0.1.2` registry commands should remain unchanged until publishing.
2. Check all 16 crates' public API documentation against corresponding `crates/*/src`, generated examples and the candidate cargo manifests. Review accepted flags and signatures, error codes, supported backends and minimum Rust version.
3. Independently inspect authentication, cookies, CSRF, database migrations and destructive actions. These areas require a security-aware source review rather than an AI paraphrase.
4. Recheck the Django capability matrix, ORM/admin/forms coverage and the difference between **implemented**, **experimental**, **planned** and **unsupported**. No unverified claim of feature parity.
5. Check version-sensitive operational instructions for Docker, PostgreSQL, reverse proxy, backup/restore and release rollback against a disposable deployed candidate.
6. Review old internal feature milestones and archive documents so readers do not mistake historical `v0.x` milestones for published Cargo versions.
7. Upgrade to **strict site validation** once all external-doc links, anchors and warnings are resolved, and verify code examples and external links separately. Missing-page warnings must never be suppressed as proof of completeness.
8. Assess the docs theme's lifecycle: **Material for MkDocs 9.x receives support through May 5, 2027**, according to its own build warning. Plan a reviewed transition to Zensical or another maintained theme in time rather than silently relying on a retired dependency.

This backlog is a **release blocker under #92/#93** until signed off. The new site is a foundation, not a finished, version-certified manual.
