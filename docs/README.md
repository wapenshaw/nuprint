# NuPrint documentation

NuPrint is a **Rust + Tauri app for Windows, macOS and Linux** with extensible printer protocols and model profiles. The first implementation (workspace version 1.0.0) covers the SC03h end to end; the original Python app is kept in `poc/python` as the verified parity reference. These documents mix implemented behavior and planned work; each marks which is which.

| Document | Purpose | Status |
| --- | --- | --- |
| [User guide](user-guide.md) | Supported printers, installing, the desktop app and troubleshooting | Implemented |
| [Building from source](building.md) | Requirements, desktop and CLI builds, checks, the Slint app and the repository layout | Implemented |
| [Product specification](app-spec.md) | Cross-platform scope, screens, workflows, milestones and acceptance criteria | V1 scope partly implemented; see validation |
| [Rust + Tauri specification](rust-tauri-spec.md) | Stack, workspace, frontend/backend contracts, OS requirements and migration gates | Implemented on Windows (dev build) and macOS (dev and packaged debug builds); release packaging and Linux pending |
| [Printer support architecture](printer-support.md) | Transport/protocol/profile boundaries, discovery, capabilities and new-model acceptance | Implemented for one protocol: SC03h verified, 34 experimental profiles |
| [Technical specification](technical-spec.md) | Python baseline and the Rust persistence/render/job-state contracts | Implemented + remaining proposals |
| [Printer protocol](protocol.md) | Model-specific wire format, GATT endpoints, packet samples and evidence | APK-derived + one-unit hardware verification |
| [APK printer catalog](apk-printer-models.md) | Model names, prefixes, dimensions and app-list membership from the supplied APK | Research candidates; SC03h only verified |
| [Printer interaction](printer-interaction.md) | Exact connection/print commands (Rust CLI), Python PoC API and troubleshooting | Implemented |
| [Validation and release plan](validation.md) | Automated, hardware, usability and packaging acceptance tests | Python and Rust results + planned coverage |
| [Engineering backlog](backlog.md) | Deferred refactors, performance work, wording decisions and Rust/Tauri best-practice review findings | Proposed |
| [Rust follow-up plan](rust-follow-up-plan.md) | Sequenced validation, runtime performance, dispatcher recovery, module, lint and doctest work after the completed review fixes | Steps 0–8 landed or deferred with reasons; see its progress table |
| [Baseline performance](performance.md) | Release-mode, hardware-free measurements of previews, admission, storage, logging and shutdown, with the benchmark harness command and the results after bounded rendering and the log writer | Measured 2026-10-29 on Windows |
| [Contributing](../CONTRIBUTING.md) | Development setup, checks, printer reports and rules for protocol changes | Active |
| [Security policy](../SECURITY.md) | How to report vulnerabilities privately | Active |
| [Agent instructions](../AGENTS.md) | Repository rules for Codex and other coding agents | Active |
| [Rust language server for Codex](codex-rust-lsp.md) | Project MCP configuration, rust-analyzer prerequisites and verification | Configured |
| [Rust language server for Claude Code](claude-rust-lsp.md) | Native Rust plugin, prerequisites and verification | Configured |
| [Printer-development skill](../.claude/skills/sc03h-printer/SKILL.md) | Reusable workflow for protocol and hardware work | Active |

Start with [the user guide](user-guide.md) to run the app and [building from source](building.md) to build it. Use [the interaction guide](printer-interaction.md) to connect from the CLI. Read [the Rust + Tauri specification](rust-tauri-spec.md) before changing the app and [the printer support architecture](printer-support.md) before adding models.

Evidence labels used throughout:

- **Implemented:** behavior present in the committed source.
- **Hardware verified:** observed on the owner's SC03h-3DE8; physical claims also require a paper observation. Say which app (Python PoC or Rust) produced the observation.
- **APK-derived:** recovered from the supplied Android app, not independently exercised in every condition.
- **Proposed:** target behavior requiring implementation and validation.

Specification date: 2026-10-12. Commit hashes in older evidence records referred to pre-release history, which was squashed into the first public commit; the records name the date and version instead.

## Deferred future work — outside V1

[Windows system-printer integration](future/windows-printing.md) preserves research, tradeoffs and possible acceptance criteria for a separate later task. No release is assigned. It is excluded from V1 architecture, implementation milestones and release gates.
