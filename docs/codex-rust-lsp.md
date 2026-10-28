# Rust language server for Codex

NuPrint uses **rust-analyzer through Serena's local MCP server** for Rust symbol
lookup, file outlines, references, declarations, implementations and diagnostics.
The executable alone does not
expose LSP tools to Codex; the bridge is configured in
[`.codex/config.toml`](../.codex/config.toml). This is a repository-scoped setup,
not a change to the user's global Codex configuration.

## Setup

1. Install Rust with rustup and [uv](https://docs.astral.sh/uv/getting-started/installation/)
   and make `rustup`, `cargo` and `uvx` available on the PATH inherited by Codex.
2. From the repository root, run `rustup show`. The pinned toolchain file includes
   `rust-analyzer` and `rust-src` alongside rustfmt and Clippy. An existing checkout
   can also run `rustup component add rust-analyzer rust-src`.
3. Set `mcp_servers.rust_lsp.cwd` in `.codex/config.toml` to the **absolute checkout
   root on this machine**. The committed value is the maintainer's checkout. This explicit
   directory handles desktop clients that start MCP outside the project; update
   it after moving/cloning the repo, including on macOS/Linux.
4. In Serena's user configuration (`~/.serena/serena_config.yml`, generated on
   first launch), add this exact checkout root to `trusted_project_path_patterns`.
   This enables the repo's language-server settings, including its isolated
   target directory. Preserve any existing trust entries.
5. Open this trusted project in Codex and restart/reconnect its MCP servers (or
   start a fresh Codex session). In the CLI, `/mcp` should show `rust_lsp`.

The first launch downloads Serena and its Python 3.13 runtime/dependencies using
uv (the explicit Python version avoids unsupported dependency builds). Serena
is pinned to upstream commit `4aa0d9fa40df457b14779a9789fbae8ac71d886d`; change that
pin deliberately and recheck the MCP tool names when upgrading. It uses the
rust-analyzer belonging to this checkout's Rust toolchain. No API key is needed.

## What is configured

- [`.serena/project.yml`](../.serena/project.yml) selects the Rust LSP backend and
  respects gitignored artifacts. The Python PoC, frontend source and skills are
  excluded from Serena's source index; this setup adds Rust tooling only.
- Codex exposes symbol lookup, references, outlines and file diagnostics, plus
  the bridge's initialization/project-selection tools. Serena is read-only;
  Codex's normal tools still handle source changes.
- Cargo build scripts and procedural macros are enabled for accurate workspace
  analysis. They can execute local build scripts, as normal Cargo development
  does. Language-server build artifacts go to ignored `target/rust-analyzer`,
  separate from the app's usual build directory, and Cargo uses `--locked`.
- The bridge uses stdio, with its browser dashboard and GUI log window disabled.
  It runs while the client is connected; no OS service or printer integration is
  installed.
- Serena caches/memories are ignored; only its project settings are shared.
  Standard-library sources come from `rust-src`.

The core and desktop host are part of one Cargo workspace. Native desktop
dependencies still need the normal build prerequisites and PDFium from
`scripts/fetch-pdfium.*`. Analysis on Windows does not validate macOS/Linux
conditional code or packaged builds.

## Verify and troubleshoot

Run `rust-analyzer --version` and `codex mcp get rust_lsp` from the repo. These
check installation/configuration; verify actual LSP access by asking Codex:

> Use rust_lsp to find MonoBitmap in crates/nuprint-protocol/src/bitmap.rs, then
> find its references in the other crates.

If the tools are missing in an existing conversation, reconnect MCP or start a
new session. If Serena has no project active, call `activate_project` with this
checkout's absolute path. If it cannot find uvx or rustup, restart the Codex app
after correcting PATH. Initial analysis can take longer while Cargo builds the
isolated cache. Inspect diagnostics with `get_diagnostics_for_file`; retain the
normal fmt, Clippy and test checks from AGENTS.md.

Verified on Windows on 2026-10-13 using the configured stdio launch: MCP
initialization and all nine configured tool names, the active NuPrint Rust
language server, the `MonoBitmap` outline/definition, references into core and
render crates, and a file-diagnostics request for `bitmap.rs`. The test client
then closed its server. This verifies the bridge, not a full workspace build or
the current chat's ability to hot-load newly configured tools.

References: [Codex project-scoped MCP configuration](https://learn.chatgpt.com/docs/extend/mcp?surface=cli),
[Serena's Codex integration](https://oraios.github.io/serena/02-usage/030_clients.html#codex-cli-and-app),
and [rust-analyzer configuration](https://rust-analyzer.github.io/book/configuration.html).
