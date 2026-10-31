# How the NuPrint app behaves

The [user guide](user-guide.md) covers how to use NuPrint. This page records the details behind it: which printers are offered and why, how the queue keeps jobs safe, and what updates, diagnostics and the log do.

## Printers

- **SC03h** (lowercase, names starting `SC03h-`): verified on Windows, macOS and Linux.
- **Experimental models** are printers whose settings in the vendor's Android app are identical to the SC03h's. NuPrint shows them with an **Experimental** badge and asks you to confirm before you save one (in the CLI, pass `--allow-experimental`). They use the SC03h's tested settings but match only their own exact name, and NuPrint sends nothing until the printer shows the expected Bluetooth print service. [The APK printer catalog](apk-printer-models.md) lists these models and the evidence behind them.
- **Everything else**, including uppercase names such as `SC03H-` or `X6H-` (which use different heat settings), shows up as an unknown device, and NuPrint never sends it print commands.

If NuPrint doesn't list your model, run `nuprint inspect --device NAME --output inspect.json`. It records the printer's Bluetooth services without sending it any printer command.

[Validation](validation.md#current-status) records what has been tested on which system.

## Installers

Check a download against the release's `SHA256SUMS` if you like. The Windows installer installs for the current user without admin rights, and the `.deb` installs BlueZ as a dependency. To confirm that an installer was built by NuPrint's release workflow, see [check a download](code-signing.md#check-a-download).

The `nuprint` CLI has no installer. You build it from source (see [building from source](building.md)).

## NuPrint Slint

Each release also includes **NuPrint Slint**, the same app drawn natively with Slint instead of in a web view. Its files are `nuprint-slint_<version>_x64-setup.exe`, `nuprint-slint_<version>_aarch64.dmg`, `nuprint-slint_<version>_x64.dmg` and `nuprint-slint_<version>_amd64.deb`. It uses the same printing engine, installs alongside NuPrint and keeps its own printers and queue. It's newer and less tested: its screens have been checked on Windows, printing from it hasn't been tested on paper yet, and it lacks two options (see the [Slint notes](building.md#slint-app)). It has the same screens as NuPrint.

## The window

NuPrint has a single window. The queue slides in over the preview and settings open on top, so neither one rearranges the window. **Check connection** is in the printer menu and beside each saved printer; it reads the printer's status without moving any paper. The printer light appears in the toolbar, the printer menu and the printer list.

The preview is the exact bitmap that will print, repeated for each copy and followed by the extra feed as blank paper. **Save as PNG** writes that exact bitmap to a file.

## The queue

The status bar shows the latest job's state and its data transfer. The outcome stays there until the next message replaces it or you press **Dismiss**. In the **Queue** panel each job shows its receipt.

Waiting, failed and uncertain jobs keep their print data, so you can print them again. When a job completes, NuPrint deletes its print data but keeps its details and a preview of its first page for 30 days, or until you remove the job or clear the history. That preview can show what you printed, so remove the job if that matters to you. A job without a preview shows blank paper of the length it used.

Jobs are saved in a durable queue, so they survive restarts. If the app stops partway through a transfer, the job is marked **uncertain** and is never resent automatically. "Transfer complete · printer reports ready" means the printer accepted the data and reported that it's ready. It doesn't mean anyone has seen the paper.

If NuPrint's own queue worker fails (an internal error, not a printer problem), the queue shows **Queue stopped** and the status bar says so. From then on NuPrint starts no job, accepts no new one, refuses **Resume queue** and stays off Bluetooth, even for **Check connection**, because it can't prove that the failed worker let go of the printer. Waiting jobs are kept. A job the worker had already claimed is marked **uncertain** and is not resent. If NuPrint can't save that mark, the next start marks the job uncertain instead. **Restart NuPrint** recovers: waiting jobs then come back on hold until you resume them. NuPrint doesn't restart the queue worker while the app is running.

## Advanced printer settings

**Settings → Printers → Advanced** is available for the SC03h and experimental printers. It edits heat for each darkness level, print speed, chunk size, pacing and every other tunable setting. The common ones have their own boxes, and the rest are in the JSON below them. Edits belong to that saved printer on this computer. They're always shown as **Experimental** and never change the built-in profile. You can't edit how the printer is recognised (its name match and protocol). **Reset to defaults** restores the built-in values, and **Export profile…** saves the settings as a complete profile file. A wrong value can make a print fail or waste paper.

## Appearance

Light and dark also apply to the window's title bar. While NuPrint is open, the chosen icon replaces the little printer in the Dock on macOS, or on the window and taskbar elsewhere. Finder, Launchpad and shortcuts keep the little printer. NuPrint remembers all three choices.

## Updates

NuPrint checks for updates only when you press **Check for updates**, or at every start if you turn on **Check for updates when NuPrint starts**, which is off until you do. A check asks GitHub, where NuPrint is published. If a check at start finds a new release, the status bar tells you.

When a newer release exists, the Updates page shows what changed. **Install and restart** downloads it, checks its [signature](code-signing.md#updates), installs it and opens NuPrint again. Waiting jobs are kept, and none of them starts while the update installs. The button is unavailable while a job is printing, because closing the app would cancel the job. On Windows the installer shows its progress; if it can't start, NuPrint tells you and starts printing again once you restart it. On Linux the system asks for your password to install the new package. **Release page** opens the release on GitHub.

NuPrint Slint updates itself the same way on Windows and macOS. On Linux it shows the new release, and **Download** opens the release page, where you can get the new `.deb`.

Updating from inside the app works from the first release that has this page. To update an earlier version, install the new release's installer.

## Diagnostics and the log

Unless you choose to include them, a diagnostics export redacts device names, addresses, job titles, the names you gave edited printer settings, and backend error details, including those in the log. Documents are never uploaded anywhere.

NuPrint also keeps a log file, `logs/nuprint.log`, in its data folder, with the previous 2 MB in `nuprint.log.1`. A background thread writes the file, so a crash or a forced kill can lose the latest lines. Those lines are written when the app quits normally and before a diagnostics export. If lines arrive faster than the disk accepts them, or the file can't be written, NuPrint drops informational lines before warnings and errors, and the log says how many it lost. The on-screen log and the diagnostics export are unaffected. The log file itself is not redacted.

## Bluetooth

NuPrint never pairs or unpairs devices. It follows the printer's buffer pause and resume signals. Cancelling stops any further transfer, but rows already in the printer's buffer may still print. On macOS, **Open Bluetooth settings** goes to Privacy & Security → Bluetooth.

For problems you see from the command line, see [diagnose by symptom](printer-interaction.md#diagnose-by-symptom).
