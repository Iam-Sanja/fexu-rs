# Documentation maintenance and version policy

[Documentation home](index.md) · [Roadmap](roadmap.md) · [Release candidate gates](release-candidate-0.2.md)

## Publication status

The code and documentation on `main` describe **unreleased development**, unless a page explicitly identifies a versioned published contract. As of 2026-11-20, **0.1.2** is the latest published Cargo workspace release; **`main` declares `0.2.0`**, while **0.2.0 is planned, not published**. The CLI generator keeps its registry default at published 0.1.2 until the coordinated release pin is updated. README crate badges show actual crates.io releases. Never rewrite historical references to 0.1.2 as if they were 0.2.0.

The public user manual uses Markdown in `manual/` and builds with MkDocs (`mkdocs.yml` sets `docs_dir: manual`). Engineering notes stay under `docs/`. `mike` captures an immutable, version-specific rendered site on a `gh-pages` branch. This can give readers paths such as `/fexu-rs/0.2.0/` and `/fexu-rs/stable/`, with a version selector. Creating a version or changing the `stable` alias is a **separate release action**, never a side effect of merging a pull request.

## Editorial rules

1. Distinguish **published 0.1.2**, **unreleased main**, and **candidate 0.2.0** on every version-sensitive page.
2. Prefer verified source paths, test names and output over general capability claims. Clearly label planned, experimental, unsupported and SQLite-only behavior.
3. Update the README and roadmap in the **same reviewed release PR** as the workspace version, internal dependencies, generator pins, lockfile and actual release notes.
4. Use cross-page Markdown links relative to `docs/`. Preserve historical records and explicitly date audit snapshots; do not rewrite old incident logs as current evidence.
5. Before release, recheck CLI commands, all 16 crate descriptions, examples and SDK/API names, security limitations, supported Rust/database versions, browser/admin behavior, dependency versions and Django comparison claims against the candidate commit.
6. Add or update a **version-specific upgrade guide** and release notes for each release. A pre-1.0 minor update may require user source changes. Document irreversible migration limitations and rollback procedures.
7. An AI-generated or AI-reviewed claim is **not evidence** by itself. Run static document checks and real examples, and have code-sensitive claims reviewed against source and tests.

## Local preview and validation

From the repository root:

```bash
python -m pip install -r requirements-docs.txt
python scripts/audit-documentation.py
mkdocs build --strict
mkdocs serve
```

The CI build uses `mkdocs build --strict`: site warnings must fail the job instead of being reported as success. Structural validation still cannot prove that code examples compile, external links remain reachable, or framework claims match source. Record those reviews separately before release.

## Development preview on GitHub Pages (explicit manual deployment)

A standalone `.github/workflows/docs-pages-preview.yml` workflow can publish the **unreleased** `main` manual to GitHub Pages. This is a development preview, not a frozen release-specific manual and not approval to publish `0.2.0`.

1. In the repository's **Settings → Pages → Build and deployment**, select **GitHub Actions** as the source. The repository owner must do this once; the documentation CI cannot change repository Pages settings.
2. In **Actions**, open **Publish documentation preview (manual)**, choose **Run workflow** on `main`, and inspect the completed deployment. The expected base URL is `https://iam-sanja.github.io/fexu-rs/` once GitHub Pages is enabled and the run succeeds.
3. The workflow checks the docs, runs `mkdocs build --strict`, and deploys the `manual/` content using the Pages artifact. It has no crates.io credentials, and does not publish a crate, create a Git tag, or alter a stable alias.
4. GitHub Pages workflows need the repository's **GitHub Actions** Pages source. The older `mike deploy --push` instructions below use a **different Pages source** (`gh-pages` branch); never switch sources implicitly. Decide the final release-specific `mike` publishing mechanism in a separate approved change before the release, rather than assuming both methods can run simultaneously.

The preview is updated **only by manually running the workflow**. No push or pull request triggers a deployment. Readers must use the documentation version guide to distinguish published `0.1.2` from the unreleased source version.

## Publish versioned docs — only after a signed-off release

For the legacy `mike deploy --push` procedure below, GitHub Pages must first be configured by a maintainer with the `gh-pages` branch as its source. This is **not** the Pages source used for the development-preview workflow above; switch only as part of a separately reviewed release-publishing plan. The following is a **manual maintainer action**, not something triggered automatically by this repository:

```bash
# From the reviewed release tag checkout, after verifying that
# v0.2.0 is actually published and approved:
python -m pip install -r requirements-docs.txt
python scripts/audit-documentation.py
mkdocs build
mike deploy --push --update-aliases 0.2.0 stable
mike set-default --push stable
```

Only run these instructions with credentials authorized to write `gh-pages`. First publish the release, then publish its frozen documentation version. The build on a given tag represents that exact release; do not publish mutable `main` content under `0.2.0`.

The candidate documentation workflow builds and audits but deliberately has **no permission to deploy GitHub Pages**. This is an opt-in publishing step; no public site URL is asserted to be active until GitHub Pages is actually configured and a version deployed.

## AI-assisted documentation review

The static audit provides file counts, links, code-fence balance and versions that need review. It cannot determine whether a statement about a Rust API is correct. For the full pre-release pass, group the pages by domain (models/ORM, migrations, auth/security, admin/forms, developer tooling, deployment/release, examples), verify each against its implementing code or tests and record results and deferred work in an issue/PR. Treat obsolete claims as bugs and link corrective commits. See [the dated 2026-11-14 audit](documentation-audit-2026-11-14.md) for a previous snapshot, not a current guarantee.
