# Contributing to NuPrint

Thanks for helping. NuPrint talks to real hardware that burns real paper, so the project is careful about evidence: what was observed on a printer, what was inferred, and what is only proposed. Most of these guidelines follow from that.

## Ways to help

- **Try an experimental printer.** If you own one of the [experimental models](docs/apk-printer-models.md), print one short strip and [open a printer report](https://github.com/wapenshaw/nuprint/issues/new?template=printer_report.yml) with a diagnostics export (**Settings → Diagnostics**, redacted by default) and what the paper looked like. A report like this is the only way a profile can be promoted.
- **Report a model NuPrint doesn't list.** Run `nuprint inspect --device NAME --output inspect.json`. It records the Bluetooth services and sends no printer command. Attach the file to a printer report.
- **Fix bugs and improve the apps.** Look for issues labelled `good first issue`, or open an issue first if you plan a larger change.

## Development setup

You need Rust (rustup installs the pinned toolchain from `rust-toolchain.toml`), Node.js 26 or newer, and on Linux the [Tauri system packages](https://tauri.app/start/prerequisites/) plus BlueZ development headers (`libdbus-1-dev`). Fetch PDFium once, because PDF tests and PDF preview need it:

```sh
./scripts/fetch-pdfium.sh        # Windows: .\scripts\fetch-pdfium.ps1
```

Then run the app with `cd apps/tauri && npm ci && npx tauri dev`, or the CLI with `cargo run -p nuprint-cli -- --help`. [docs/building.md](docs/building.md) has the full command list. [docs/README.md](docs/README.md) indexes the architecture, protocol and validation documents.

## Before you open a pull request

Run the offline checks from the repository root. None of them needs a printer.

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
(cd apps/tauri && npm ci && npm run typecheck && npm test)
git diff --check
```

- `cargo test -p nuprint-core` regenerates the TypeScript bindings in `apps/tauri/src/bindings`. Commit them together with the Rust type change.
- For UI changes, run `npx tauri dev` and look at the screens you changed at the default window size. Include a screenshot in the pull request.
- Update the documentation that describes current behaviour in the same pull request as the behaviour change. Keep planned work clearly marked as planned.

## Commits and pull requests

- Branch from `master`, named `<type>/<kebab-summary>`, such as `fix/queue-cancel-race`. Nobody commits to `master` directly; it only moves through pull requests.
- Commits follow [Conventional Commits](https://www.conventionalcommits.org): `<type>(<scope>): <subject>`, imperative and lowercase, no trailing period, 72 characters at most. Types in use are `feat`, `fix`, `perf`, `refactor`, `docs`, `test`, `ci` and `chore`. The scope is a crate or area (`protocol`, `transport`, `render`, `core`, `cli`, `desktop`, `slint`, `profiles`), or `deps` for a dependency update. The body explains **why**; the diff already says what. One logical change per commit, and never a co-author trailer.
- `feat`, `fix`, `perf` and `refactor` commits become the changelog, so write their subjects for someone who uses NuPrint. A dependency update is `chore(deps)` whatever it updates, so it cuts no release on its own.
- **Pull requests are merged by rebase.** Merge commits and squashing are disabled: a merge commit would put the pull request title into the history and a squash would collapse a pull request into one changelog line.
- A pull request needs the approval of a code owner ([.github/CODEOWNERS](.github/CODEOWNERS)) and passing CI to merge. CI runs on its own only for the maintainer's pull requests. Anyone else's waits for approval: a fork's run is approved from the pull request, and a branch in this repository (Renovate's, a collaborator's) waits at the `Approval` job.
- Fill in every section of the [pull request template](.github/pull_request_template.md): a Summary that shows the change, Evidence from before and after, and the Merge Danger.
- Keep a pull request to one thing. A refactor, a feature and a formatting sweep are three pull requests.

## Versions and releases

[release-please](https://github.com/googleapis/release-please) owns versions, tags and `CHANGELOG.md`. Do not edit any of them by hand. From the Conventional Commits on `master` it keeps a release pull request open that bumps the version in `Cargo.toml`, `Cargo.lock`, `apps/tauri/package.json`, `package-lock.json` and `tauri.conf.json`; a `feat` bumps the minor version, a `fix` or `perf` the patch, and a `!` or `BREAKING CHANGE:` footer the major. Merging it tags `vX.Y.Z` and opens a draft release. The [release workflow](.github/workflows/release.yml) then builds the installers (Windows NSIS setup, macOS disk images for Apple silicon and Intel, Linux `.deb`), attaches them with `SHA256SUMS` and publishes the release.

## Printer and protocol changes

Read [docs/protocol.md](docs/protocol.md) and [docs/printer-support.md](docs/printer-support.md) first. The [agent instructions](AGENTS.md#sc03h-protocol-invariants) list the SC03h invariants that must not change without evidence. In short:

- Say where new information comes from: **observed on hardware**, **derived from the vendor app**, **inferred** or **proposed**. A successful Bluetooth write is not proof that paper came out.
- Never widen an existing profile's name match as a shortcut to supporting another model. `SC03h-` and `SC03H-` are different printers.
- Golden vectors in `fixtures/` are generated only by the Python reference (`poc/python/tools/gen_fixtures.py`), and only when the reference behaviour itself changes. `crates/nuprint-protocol/tests/python_parity.rs` must keep passing.
- Record hardware results in the format in [docs/validation.md](docs/validation.md#evidence-record-format).

## What must never be committed

Vendor APKs, decompiled vendor sources, PDFium binaries, build output, logs, and any personal documents or diagnostics that have not been redacted. The `.gitignore` covers the usual locations (`apks/`, `.tools/`, `vendor/`, `target/`, `output/`). Commit the behaviour you implemented independently, with short observations and test vectors, and nothing copied from the vendor app.

## Coding agents

The repository is set up for AI coding agents. [AGENTS.md](AGENTS.md) holds the rules they follow. Human contributors may find it a useful summary of the project's constraints too.

## Licence

By contributing you agree that your contribution is licensed under the [MIT License](LICENSE), the same as the project.
