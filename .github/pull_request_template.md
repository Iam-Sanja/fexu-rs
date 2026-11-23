## Summary

<!-- What changed and why. -->

## Related issue

<!-- Issue number, or "none". -->

## Type of change

- [ ] Bug fix
- [ ] Feature
- [ ] Documentation
- [ ] Refactor
- [ ] Test
- [ ] CI or repository maintenance

## Implementation notes

<!-- Relevant modules, commands, and decisions. -->

## Testing performed

<!-- Commands you ran and their results. Do not claim a check that did not run. -->

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] `cargo build --workspace`

## Documentation

- [ ] Updated the relevant `manual/` pages, or explained `Docs-Impact: none — ...` in the PR body
- [ ] Updated `README.md` for public feature/quickstart changes, or explained `README-Impact: none — ...`
- [ ] Updated roadmap/changelog for feature, fix or release status changes
- [ ] Synchronized all 16 Cargo crate versions, internal dependencies, lockfile and generator pins if the declared version changed

## Migration impact

<!-- New migration, edited migration, or none. Applied migrations must not be rewritten. -->

## Breaking-change assessment

<!-- API, route, settings, scaffold, or job behavior. Internal milestone labels are not Cargo versions. -->

## Security considerations

<!-- Auth, sessions, password reset, migrations, job payloads, or "none". -->

## Reviewer checklist

- [ ] The change matches the summary
- [ ] Tests cover the behavior that changed
- [ ] No secrets, local databases, or unrelated rewrites are included
- [ ] Existing generated projects remain compatible, or the break is explicit
