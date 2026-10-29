# Changelog

release-please writes this file from the Conventional Commits on `master`;
do not edit it by hand. Versions follow semver and match the workspace
`version` in `Cargo.toml`, `apps/desktop/package.json` and
`apps/desktop/src-tauri/tauri.conf.json`.

## [1.0.1](https://github.com/wapenshaw/nuprint/compare/v1.0.0...v1.0.1) (2026-10-29)


### Bug Fixes

* **core:** back up the store from sqlite before upgrading it ([3dc8726](https://github.com/wapenshaw/nuprint/commit/3dc87260fbc445c1a5775b3596612c9b7fdcb496))
* **core:** keep connection checks from undoing printer changes ([32a6ba3](https://github.com/wapenshaw/nuprint/commit/32a6ba346a8724fbd2a8e1301bfde3f012687f5c))
* **core:** keep logging after a panic poisoned the log ring ([f0e80ca](https://github.com/wapenshaw/nuprint/commit/f0e80ca685133a56c782acbb7b94005de563c536))
* **core:** keep printer names out of redacted diagnostics ([5a06aa4](https://github.com/wapenshaw/nuprint/commit/5a06aa43c13d9c7c2449a4bc5a82e80cca04d26c))
* **core:** publish edited profiles only after they are saved ([a73f920](https://github.com/wapenshaw/nuprint/commit/a73f9203e7f92370f0da0dbfeb8e572b4fcd154a))
* **core:** recover the pdf engine lock after a panic ([0236534](https://github.com/wapenshaw/nuprint/commit/023653492159367e3dc6101fefa6be6cef6f0729))
* **core:** refuse profile sizes and waits that cannot print ([4a86b47](https://github.com/wapenshaw/nuprint/commit/4a86b4766baa1f7472e7d867aa5c3ce5e9f8aaa6))
* **core:** remove print data of jobs that were never saved ([bd25026](https://github.com/wapenshaw/nuprint/commit/bd25026d37b1c1b388694022c0eb9ab8cfadc688))
* **core:** report lines the log file lost in the in-memory log ([1b9320b](https://github.com/wapenshaw/nuprint/commit/1b9320b2d34e491ba244b3faf2420996bfb33a94))
* **core:** stop a stalled log write from hanging exit ([89ed191](https://github.com/wapenshaw/nuprint/commit/89ed191af7e9b467308d9e51370a6cec659f3bbd))
* **core:** stop the queue when the dispatcher fails ([76f63df](https://github.com/wapenshaw/nuprint/commit/76f63dfc32b5de7721896a65c983f6f472149baa))
* **core:** warn about paper only once print data may have arrived ([3e06eae](https://github.com/wapenshaw/nuprint/commit/3e06eae9ba98c1ed8c256d5482c71740428297a4))
* **desktop:** show log lines that arrive out of order in sequence ([2128ee9](https://github.com/wapenshaw/nuprint/commit/2128ee9b15e55abc0fa0834c455fffd2e736ccbb))
* recover the log writer and ble device locks after a panic ([273bbe1](https://github.com/wapenshaw/nuprint/commit/273bbe148cbd47d492e207c5a594d354188f2037))
* **slint:** keep log lines that arrive out of order ([1760772](https://github.com/wapenshaw/nuprint/commit/17607729cad1433b7080e60ebe6fe7cf09f0cf95))
* **slint:** keep the newest log lines when a burst waits for the ui ([92386e2](https://github.com/wapenshaw/nuprint/commit/92386e2b2a8f95cf687901394a705eaa8ad9289f))
* **slint:** let the settings pages and code boxes scroll ([56ede33](https://github.com/wapenshaw/nuprint/commit/56ede3330751a8edc7490656099140426f45542e))
* **slint:** reload profiles when printers change ([b96e1cc](https://github.com/wapenshaw/nuprint/commit/b96e1cc12dd20b5f42f16e6eddbbdf1d08b0bf11))
* **slint:** use gtk file dialogs so the workspace builds on linux ([1937066](https://github.com/wapenshaw/nuprint/commit/19370662c4bc5c6b4645c226790840a66c595cab))
* **transport:** disconnect when a connection attempt is cancelled ([d854921](https://github.com/wapenshaw/nuprint/commit/d854921b554ae834363eecc313db55bb0a019283))
* **transport:** restart a silent scan despite connection events ([1411aa1](https://github.com/wapenshaw/nuprint/commit/1411aa1c023b564f3e972a56d7cf0abd6b555c6d))
* **transport:** restart a windows scan that hears nothing ([a825729](https://github.com/wapenshaw/nuprint/commit/a82572931c6e51bb04f7bef347df5693a3e93b05))


### Performance

* **core:** bound preview rendering to one active and one pending ([a0a16bd](https://github.com/wapenshaw/nuprint/commit/a0a16bd8575f910d91fe930ec353bbfde6892f23))
* **core:** keep the log file open and write it from a bounded buffer ([c9ff541](https://github.com/wapenshaw/nuprint/commit/c9ff541038b6fadf4696bd32673f82ce080b847a))
* **core:** load queue pages in one query ([5b8a4d6](https://github.com/wapenshaw/nuprint/commit/5b8a4d6ec9c2b4e0ed55557b07bba45075c7e5fa))
* **core:** queue a preview without copying its pages ([18478a4](https://github.com/wapenshaw/nuprint/commit/18478a4a55100fd6f2e23febf3009b46d2d556e8))
* **core:** read and write user files on blocking threads ([39a5d0c](https://github.com/wapenshaw/nuprint/commit/39a5d0c59f87cd5a463326b128e5d19d7753bf0e))
* **protocol:** encode iprint jobs without a buffer per row ([b0a35f9](https://github.com/wapenshaw/nuprint/commit/b0a35f9b473e00acbc3f8526ce6ade73befe5058))
* **render:** composite eight-bit pictures without an rgba copy ([0a0117b](https://github.com/wapenshaw/nuprint/commit/0a0117bd188a3bafaf496152cea41f29cbed332b))
* **slint:** append log lines instead of rebuilding the log ([75ca51c](https://github.com/wapenshaw/nuprint/commit/75ca51c48733ea04ef9f3a43be2e2c4b9f1ddca1))
* **slint:** reload printers and jobs off the ui thread ([7e660aa](https://github.com/wapenshaw/nuprint/commit/7e660aafc63d61485822b4e142b1ff5163637cc0))
* **tauri:** serve preview images on a blocking thread ([3c9a33f](https://github.com/wapenshaw/nuprint/commit/3c9a33f7a63765174f787ac2f0055390f3b8bc26))


### Refactoring

* adopt cfg_select!, if-let guards and newer std helpers ([e268929](https://github.com/wapenshaw/nuprint/commit/e2689295b2c1b8c24bedccdf3246677393f83eca))
* **core:** move app info and diagnostics export into their own module ([2a16e66](https://github.com/wapenshaw/nuprint/commit/2a16e66fc1b1ed32a958a2dd390f0be86a0c18af))
* **core:** move discovery and printer management into its own module ([db8403f](https://github.com/wapenshaw/nuprint/commit/db8403faad1eb84fcdd27445348587935bd01c46))
* **core:** move job admission and queue controls into their own module ([1878bfd](https://github.com/wapenshaw/nuprint/commit/1878bfd7f2e002c55e796635ac0e2db3a84ba3ad))
* **core:** move logging and its tests into their own module ([b96b2a2](https://github.com/wapenshaw/nuprint/commit/b96b2a2446795b6b23a71fb4e686d972e8c38b8e))
* **core:** move preview and source handling into its own module ([e026dd5](https://github.com/wapenshaw/nuprint/commit/e026dd5ad317d66467f4aca99b36acba838b96e8))
* **core:** move shutdown and failure handling into their own module ([307942a](https://github.com/wapenshaw/nuprint/commit/307942ad586ace45896c0c86548d8b4375bfe67e))
* **core:** move the dispatcher and hardware claim into their own module ([aa0a935](https://github.com/wapenshaw/nuprint/commit/aa0a9352af1b814af6d96620790f17a7ccaf8c40))
* drop clones of values that are not used again ([074a4e9](https://github.com/wapenshaw/nuprint/commit/074a4e95b54cc6bfb45c1869e04c1a8854079c55))
* move the front ends to apps/tauri and apps/slint ([981b3b7](https://github.com/wapenshaw/nuprint/commit/981b3b729dc25a2a43ede272d815c21e4b952035))
* **slint:** move preview scheduling into a preview module ([79b0d1c](https://github.com/wapenshaw/nuprint/commit/79b0d1ca797d4f0e83d3a0ad2504160e55f76648))
* **slint:** move service event handling into its own module ([9fcfdb7](https://github.com/wapenshaw/nuprint/commit/9fcfdb75ccd13f27c496d4e38dbd5802de50b7ae))
* **slint:** move startup into a bootstrap module ([a5c3a90](https://github.com/wapenshaw/nuprint/commit/a5c3a90ddf745ef40200b8f9d5f02659e7ad9871))
* **slint:** move the context and its reloads into ctx.rs ([7ec3efa](https://github.com/wapenshaw/nuprint/commit/7ec3efa0a8679a01456b4c6d707302c68ba051cc))
* **slint:** move the sync functions into a sync module ([7e57d93](https://github.com/wapenshaw/nuprint/commit/7e57d938a26a89a97620ffd4f0bb11d018317374))
* **slint:** move the UI state structs into a state module ([c2dae2d](https://github.com/wapenshaw/nuprint/commit/c2dae2d3ac5180a950ae58e061f7eb48b0c157a8))
* **slint:** move view conversion into a view module ([3c4f277](https://github.com/wapenshaw/nuprint/commit/3c4f27714e0d5aa679d9da413162ace9bae70f27))
* **slint:** narrow visibility left over from the split ([3d47ff7](https://github.com/wapenshaw/nuprint/commit/3d47ff790aafc5f992102bd8f2ae2efd15ba78bb))
* **slint:** split the screen callbacks into feature modules ([a2b7525](https://github.com/wapenshaw/nuprint/commit/a2b75257d21615510be691eb3dc9ae186a91f754))

## 1.0.0 (2026-10-29)

First public release.

* Print notes, pictures and single PDF pages to the ZEITEL SC03h over Bluetooth LE from the Tauri desktop app (Windows, macOS, Linux) or the `nuprint` CLI, with an exact bitmap preview and a durable job queue.
* 34 experimental profiles for models whose vendor-app settings match the SC03h.
