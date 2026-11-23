# Documentation maintenance and version policy

[Documentation](index.md) · [Roadmap](roadmap.md) · [Release process](crates-io-release.md)

## Version status (2026-11-23)

- **Published:** Fexu **v0.2.0**, all 16 crates on crates.io ([release](https://github.com/Iam-Sanja/fexu-rs/releases/tag/v0.2.0)).
- **Next:** **v0.3.0** is planned, not released, and has no promised release date.
- **Source:** `main` can contain new development features that are *not* part of published v0.2.0. Changes requiring a new prerelease version must coordinate all workspace crates.
- **Manual:** public documentation is under `manual/` and MkDocs uses `docs_dir: manual`. Engineering history stays under `docs/`. The Pages website follows the latest merged manual on `main`; it is not a frozen 0.2.0 version. For the immutable 0.2.0 source, use [the tagged manual](https://github.com/Iam-Sanja/fexu-rs/tree/v0.2.0/manual).

## Rules for every pull request

1. **Docs with code:** when user-visible APIs, CLI, configuration, security, deployment or framework behavior changes, update the relevant `manual/` pages in the **same PR**. If truly unnecessary, give a specific `Docs-Impact: none — ...` explanation.
2. **README stays current:** update `README.md` for changes to publicly described features, installation, supported platforms and quickstarts; otherwise give a concrete `README-Impact: none — ...` explanation. Internal bug fixes do not require new marketing copy.
3. **Roadmap and changelog stay current:** amend `docs/roadmap.md` and `CHANGELOG.md` with each newly shipped feature, significant fix, deferral or change in accepted limitations. Mark plans as plans, tests as tests, and source-only features as source-only.
4. **Versions are coordinated:** all **16 publishable crates** share one workspace version and internal path dependency requirement. Check `Cargo.toml`, `Cargo.lock`, generated project pins, templates, examples and package metadata together. The first genuine v0.3.0 feature/code PR should move to an explicitly marked development prerelease (for example `0.3.0-dev.1`) only with passing tests. No crates.io publication happens for each commit, and a GitHub version is not a registry release.
5. **Evidence and security:** verify code examples and feature claims with source/tests, preserve migration safety and security limitations, and leave historical release audit evidence dated rather than rewriting it.
6. **Review gates:** CI documentation impact checks enforce a manual update or a reason, but do not prove accuracy. Run the MkDocs build and documentation audit, and review README claims in code review.

## Local validation

```bash
python -m pip install -r requirements-docs.txt
python scripts/audit-documentation.py
mkdocs build --strict
mkdocs serve
```

## Automatic GitHub Pages updates

The workflow `.github/workflows/docs-pages-preview.yml` deploys `manual/` from `main` to GitHub Pages after changes to the manual, MkDocs configuration, documentation requirements or audit script on `main`, and also supports a manual dispatch. GitHub Pages must use **GitHub Actions** as the build/deployment source and the repository must permit the `github-pages` environment. Confirm success in the [workflow runs](https://github.com/Iam-Sanja/fexu-rs/actions/workflows/docs-pages-preview.yml).

Pages always represents the **latest development manual**, not a version-frozen release archive. New 0.3.0 documentation must label unreleased features. The workflow does not publish crates, create tags or GitHub Releases, and does not manage `mike` stable aliases. A separate, reviewed versioned-docs deployment would be required to serve immutable versions with a selector.

## Release steps

Before release, test the exact candidate SHA, build all packages, review 16 crate versions and internal dependencies, run cross-version upgrade tests and tokenless preflight, and obtain explicit maintainer approval. A published GitHub Release triggers crates.io trusted publishing. After release, verify registry versions and clean any obsolete "not yet published" language without moving release tags.
