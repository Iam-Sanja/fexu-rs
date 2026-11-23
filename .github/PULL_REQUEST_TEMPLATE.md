## Summary

What does this PR change, and why?

## Verification

Which builds/tests and platforms were actually checked? Include results or links.

## Documentation impact (required before merge)

For **user-visible** changes to API, CLI flags, generator, settings, models, forms, admin, auth, deployment or dependencies, update the relevant files under `manual/` in **this PR**. Include full working examples and the version the instructions describe. The public manual is release-specific and is not the same as internal `docs/`.

- [ ] The applicable `manual/` pages were updated and checked against code **or** the no-impact explanation below is justified.
- [ ] Commands/examples were checked, or any unexecuted example is clearly identified for follow-up.
- [ ] No unreleased feature is described as already published.

For **no user-visible impact**, put one line in the PR description in this exact form (at least 20 characters of rationale):

`Docs-Impact: none — this change only adjusts internal test fixtures and does not change public behavior`

This is reviewed by the **Docs impact reviewed** check. An exemption must be accurate, not a way to bypass a necessary documentation change.

## README, roadmap and changelog (continuous maintenance)

- [ ] Updated `README.md` when public capabilities, installation or quickstart changed, **or** provided a substantive `README-Impact: none — ...` explanation.
- [ ] Updated `CHANGELOG.md` and `docs/roadmap.md` when feature/fix scope, release status or user-visible limitations changed.
- [ ] If Cargo versions changed: all 16 publishable crates, internal dependencies, lockfile and generator pins were synchronized and tested.

## Release notes / compatibility

Mention any behavior or version changes, breaking changes, migration guidance or explicitly state that there are none.
