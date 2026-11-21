# Documentation policy for pull requests

Fexu has two distinct documentation audiences:

- `manual/`: the **public versioned user manual**, hosted via GitHub Pages after an approved release. It contains installation, first-project tutorials, CLI commands, Rust code examples, settings, feature guides and deployment instructions.
- `docs/`: historical engineering references, security reviews, implementation notes and development history. These remain in Git, but are **not** built into the public website.

## Rule before merging

Every pull request that changes a publicly usable command, Rust API, configuration key, project generator, runtime behavior, database contract, admin/forms/auth behavior, or deployment instructions **must also update the relevant manual page(s) in the same PR**.

If there is no user-facing documentation impact, the PR body must contain an explicit explanation in the format:

```text
Docs-Impact: none — the internal refactor does not affect supported configuration, CLI or public APIs
```

The CI job **Docs impact reviewed** enforces the existence of a manual change or a substantive explanation for source changes. This check is deliberately *structural*: human reviewers must still verify that the manual change addresses the code change correctly.

### Reviewer checklist

1. Compare the actual changed APIs/flags/behavior with the updated manual.
2. Ensure code examples compile or are run against the candidate version.
3. Check setup instructions on supported platforms where relevant.
4. Avoid documenting an unreleased function as a published released feature.
5. Verify that new CLI options are present in the CLI reference.
6. Verify that settings changes are present in the settings reference.
7. Check `mkdocs build --strict` and local Markdown links.
8. When publishing, freeze documentation from the **release tag**, not mutable main.

The rule should be adopted as a required branch-protection check **after the currently open release-fix PRs (#231–#236) have been integrated**. Applying the new required check retroactively to PRs created before this policy would needlessly block the existing release effort; nevertheless, review their manual impact before merging.

## Version strategy

Maintain documentation on `main` for the next release, and use `mike` to publish a frozen copy from the reviewed release tag. The `stable` alias points to the latest signed-off version. Historical snapshots are never edited in place.
