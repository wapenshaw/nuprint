# Code signing policy

This page says which NuPrint downloads are signed, by what, and how to check one before you install it.

## What is signed today

| Download | Signature | How to check it |
| --- | --- | --- |
| All installers, from the next release on | GitHub build provenance attestation | [Check a download](#check-a-download) |
| Windows `NuPrint_<version>_x64-setup.exe` | Not code-signed yet; signing through the SignPath Foundation is planned | SmartScreen warns on first launch |
| macOS `.dmg` | Signed ad hoc, not notarized | Gatekeeper blocks the first launch |
| Debian/Ubuntu `.deb` | Attestation only | [Check a download](#check-a-download) |
| Updates installed from inside the apps, from the next release on | NuPrint's updater key | Each app checks it before installing; see [updates](#updates) |

The [user guide](user-guide.md#installing) shows how to get past the Windows and macOS warnings. Notarizing the macOS app needs a paid Apple Developer account and is not planned for now; see the [backlog](backlog.md#tauri-hardening-and-distribution).

## How releases are built

Every installer is built by the [release workflow](../.github/workflows/release.yml) on GitHub-hosted runners, from a tagged commit of this repository's `master` branch. Changes reach `master` only through pull requests. Nothing is built or signed on a developer's computer.

Each build job records a signed attestation for the files it produces. The attestation names the repository, the workflow, the commit and the run that built the file, and is kept by GitHub in the repository's [attestations](https://github.com/wapenshaw/nuprint/attestations) list.

When Windows signing is approved, the release workflow will sign `NuPrint.exe` and then the installer that contains it, and only after a project approver accepts each signing request. Only files built from this repository's source are signed with the project's certificate. Third-party files packaged with the app, such as `pdfium.dll` from [pdfium-binaries](https://github.com/bblanchon/pdfium-binaries), are shipped as their authors publish them and are not signed by NuPrint.

## Updates

NuPrint and NuPrint Slint can install a newer release from **Settings → Updates**. The release workflow signs each file an app installs: the Windows installer, the Linux `.deb`, and on macOS an archive of the app. It signs with a key pair kept only for updates ([minisign](https://jedisct1.github.io/minisign/) format). The private half is a secret of this repository that only the release workflow uses; the public half is built into the apps. An app installs an update only when the signature matches and names the version it was offered, so an older release can't be passed off as a newer one.

This proves that an update came from NuPrint's release workflow. It doesn't replace the Windows and macOS code signing above.

## Check a download

With the [GitHub CLI](https://cli.github.com) installed and signed in, run this in the folder you downloaded the installer to:

```sh
gh attestation verify NuPrint_1.2.3_x64-setup.exe --repo wapenshaw/nuprint
```

Use the installer's real file name. A good result lists the workflow and commit that built the file. If the command reports no matching attestation, the file was not built by NuPrint's release workflow: don't install it, and [report it](../SECURITY.md).

`SHA256SUMS` in each release lists the checksum of every installer. It catches a damaged download, but it is published next to the installers, so it doesn't prove where they came from; the attestation does.

## Team roles

| Role | Members |
| --- | --- |
| Committers and reviewers | [wapenshaw](https://github.com/wapenshaw) |
| Approvers | [wapenshaw](https://github.com/wapenshaw) |

Committers and reviewers can change the source; changes from anyone else are reviewed before they are merged. Approvers approve each release and each signing request. Every member must use multi-factor authentication on GitHub and on SignPath.

## Privacy

This program will not transfer any information to other networked systems unless specifically requested by the user or the person installing or operating it. NuPrint talks to the Bluetooth printer you choose, and to GitHub only when you check for updates: with **Check for updates** in Settings → Updates, or each time NuPrint starts if you turn that option on (it is off until you do). A check downloads the newest release's update manifest, and the update itself if you choose to install it. Nothing about you, your printers or your documents is sent. NuPrint has no telemetry or account, and it keeps its queue, settings and logs on your computer.
