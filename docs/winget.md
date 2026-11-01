# winget

NuPrint can be installed with winget in two ways. Both are called NuPrint, so give the ID; `winget install NuPrint` would ask which one you mean.

| Command | Installs | Kept up to date by |
| --- | --- | --- |
| `winget install 9NW47STT70BG` | The [Microsoft Store](microsoft-store.md) copy, from winget's `msstore` source | The Store |
| `winget install Wapenshaw.NuPrint` | The .exe installer from the GitHub release, from the community repository | `winget upgrade`, or NuPrint's **Settings → Updates** |

Like any Store copy and installer copy, the two keep separate printers and queues.

The Store copy is in winget because it's in the Store; nothing else is needed.

## The installer package

`Wapenshaw.NuPrint` lives in [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs) under `manifests/w/Wapenshaw/NuPrint`. It describes the per-user Nullsoft installer: no admin rights, product code `NuPrint`, installed to `%LocalAppData%\NuPrint`. Version 1.4.0 was submitted by hand in [microsoft/winget-pkgs#468275](https://github.com/microsoft/winget-pkgs/pull/468275) and becomes installable once a moderator merges it.

## Each release

The release workflow's `winget` job runs [komac](https://github.com/russellbanks/Komac) `update`, which copies the previous version's manifests, fills in the new installer's URL, hash and release notes, and opens a pull request on microsoft/winget-pkgs from the maintainer's fork, `wapenshaw/winget-pkgs`. winget's bots validate and install it, and a moderator merges it.

The job needs the `WINGET_TOKEN` secret: a classic personal access token of the maintainer's account with only the `public_repo` scope (GitHub → Settings → Developer settings → Personal access tokens → Tokens (classic)). This workflow's own token can't open pull requests on another repository, and komac can't open them with a fine-grained token. Without the secret, the job does nothing.

```powershell
gh secret set WINGET_TOKEN
```

`komac update` needs a version already in winget-pkgs, so the job fails until the first version is merged. A manual rebuild of a release replaces its installer and so its hash; the winget manifest for that version then needs a pull request with the new hash, for example `komac update Wapenshaw.NuPrint --version <version> --urls <installer URL> --replace <version> --submit`.
