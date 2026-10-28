# Rust language server for Claude Code

NuPrint enables Claude Code's native **rust-analyzer-lsp** plugin in
[`.claude/settings.json`](../.claude/settings.json). It provides the `LSP` tool
for symbol navigation and diagnostics after Rust edits. Claude uses the existing
rustup-managed analyzer; it does not need the Serena bridge configured for Codex.
See [Claude's code intelligence documentation](https://code.claude.com/docs/en/plugins/code-intelligence).

## Setup

From the repository root, with Claude Code and Rust on PATH:

```powershell
rustup component add rust-analyzer rust-src
rust-analyzer --version
claude plugin marketplace add anthropics/claude-plugins-official
claude plugin install rust-analyzer-lsp@claude-plugins-official --scope project
claude plugin details rust-analyzer-lsp
```

Skip adding the marketplace when it is already listed by
`claude plugin marketplace list`. The pinned `rust-toolchain.toml` includes the
analyzer and standard-library sources. Start Claude from this repository root
so rustup selects the matching toolchain and Claude loads the project settings.
Plugin installation also keeps a machine-local registration/cache; sharing the
settings file does not install the binary on another computer.

Start a fresh Claude Code session after setup. For plugin-only changes in an
existing session, `/reload-plugins` is also available. See
[plugin installation and scopes](https://code.claude.com/docs/en/discover-plugins).

## Verify and troubleshoot

`claude plugin details rust-analyzer-lsp` should list **one LSP server** named
`rust-analyzer`; `claude plugin list` should show it enabled at project scope.
Then ask Claude:

> Use the LSP tool to find MonoBitmap in crates/nuprint-protocol/src/bitmap.rs
> and its references in the other crates.

If it fails to launch, check `/plugin` → Errors and ensure `rust-analyzer` and
`cargo` are available in the shell that launches Claude. Native desktop
dependencies and PDFium still require the normal development prerequisites.
The official plugin uses rust-analyzer's default Cargo configuration, including
the normal build directory. If a running Windows app locks PDFium during
analysis, close that app and retry. Codex's isolated analyzer cache settings
apply only to its Serena server.

On this machine, the earlier `rust-analyzer-lsp@anthropic-plugin-directory`
installation listed zero LSP servers: its cached directory contained only a
README and license, and its directory entry lacked the server definition.
Switching to the official Git marketplace preserved the plugin settings and
registered the missing server. If the same installation conflict occurs, use
the CLI's suggested replacement command:

```powershell
claude plugin install rust-analyzer-lsp@claude-plugins-official --replace --scope project
```

Verified on Windows on 2026-10-14 with Claude Code 2.1.296 and plugin 1.0.0:
project enablement, one registered Rust server, and a direct stdio smoke check
using the plugin's command. Rust-analyzer 1.98.0 initialized successfully and
returned `MonoBitmap` from a document-symbol request. The check then shut down
the server. This verifies registration and server operation; an in-session
Claude `LSP` call, edit diagnostics and a full Cargo build were not tested.
Keep the normal fmt, Clippy and test checks from `AGENTS.md`.
