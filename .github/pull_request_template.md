## Summary

<!--
The smallest view that makes the point: pseudocode, a call tree, a diff
sketch or a screenshot, each beside one or two lines of why.
-->

Closes #

## Evidence

<!--
A before and an after: a failing then passing test, the screen before and
after, a preview PNG. For printing or Bluetooth changes: which printer and
OS, what was sent, and what the paper looked like, or "not tested on
hardware".
-->

- **Before:**
  **After:**

## Merge Danger

<!--
A two-way door is cheap to walk back; a one-way door is not: a store
migration, a change to wire bytes or a profile's values, a renamed release
asset, anything that could send print commands to a device that did not get
them before.
-->

**Door:** <!-- one-way or two-way -->

**Blast Radius:** <!-- one word: what could break, and for whom -->

## Before merging

- [ ] `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`
      and `cargo test --workspace --locked` pass locally.
- [ ] `npm run typecheck` and `npm test` pass in `apps/desktop` (frontend changes).
- [ ] One logical change per commit, Conventional Commits, no co-author
      trailer. **PRs are rebased, never squashed or merged**, so the commit
      messages are what lands in the history and in the changelog.
- [ ] Tests land with the behaviour they cover.
- [ ] `CHANGELOG.md`, the tags and the versions in `Cargo.toml`, `Cargo.lock`,
      `package.json` and `tauri.conf.json` are untouched; they belong to release-please.
- [ ] Documentation of current behaviour is updated.
- [ ] This pull request is one thing.
