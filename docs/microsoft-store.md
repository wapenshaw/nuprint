# Microsoft Store

NuPrint for Windows is also offered through the [Microsoft Store](https://apps.microsoft.com/detail/9NW47STT70BG) (product ID `9NW47STT70BG`), as an MSIX package that the Store signs. winget installs the same copy with `winget install 9NW47STT70BG`; see [winget](winget.md). A Store install shows no SmartScreen warning, is updated by the Store (Settings → Updates says so and never checks GitHub), and keeps its own saved printers and queue in its package folder, apart from an installer copy's; Windows removes them when the app is uninstalled. [Building](building.md#microsoft-store-package) has the packaging commands.

## Package identity

`apps/tauri/src-tauri/msix/identity.json` holds the three values from Partner Center, **Product management → Product identity**: `Package/Identity/Name`, `Package/Identity/Publisher` and `Package/Properties/PublisherDisplayName`. They are not secret. The Store rejects a package whose identity differs from the reserved product's.

## Submitting a release

The release workflow's `store` job submits each release on its own once [automatic submission](#automatic-submission) is set up. It uploads the package as a draft, sets **What's new** from the release's changelog entry (as plain text, without commit links), and submits the draft for certification. Certification usually takes up to three business days, and the Store publishes the update when it passes. A manual rebuild of an existing release never resubmits.

If the job fails after uploading, the draft stays in Partner Center: finish it there from step 4 below. Without the secrets, submit by hand:

1. Download the package the release workflow built: the run's `microsoft-store` artifact, for example with `gh run download <run-id> -n microsoft-store`. It isn't attached to the release, because Windows installs an unsigned MSIX only from the Store.
2. In Partner Center, open NuPrint and **Start update** (the first time, **Start your submission**).
3. **Packages**: upload `NuPrint_<version>_x64.msix`. Partner Center checks the identity and version; a version must be higher than the last one submitted.
4. **Store listings → English**: paste "What's new in this version" from the release's changelog entry.
5. **Submit to the Store**. Certification usually takes up to three business days.

## Automatic submission

The `store` job signs in to Partner Center as a Microsoft Entra application, through the [Microsoft Store Developer CLI](https://learn.microsoft.com/en-us/windows/apps/publish/msstore-dev-cli/github-actions). Microsoft supports this for free apps. To set it up once:

1. Partner Center's user management accepts only a work account in a Microsoft Entra tenant. If the Partner Center account is a personal Microsoft account (Outlook or Hotmail), create one: in the Azure portal, open the account's directory (an Azure sign-up makes one, `<name>.onmicrosoft.com`), add a user under **Manage → Users → New user** with the **Global Administrator** role, and sign in as that user once to set its password.
2. In Partner Center, open **Account settings → Organization profile → Tenants**, choose **Associate** and sign in with that work account.
3. Open **Account settings → User management** (signing in with the work account when asked) → **Microsoft Entra applications → Create Microsoft Entra application**. Give it a name and the **Manager** role. Its **Reply URL** is required but unused, because the job signs in with a client secret rather than through a browser; `https://nuprint.app` will do.
4. On the application's page, note the **Client ID**, and use **Add new key** to make a client secret. Partner Center shows the secret only once, and it expires; make a new one before then. The **Tenant ID** is on the directory's overview in the Azure portal, and the **Seller ID** is under **Account settings → Legal info**, in **Publisher IDs**.
5. Save the four values as repository secrets:

   ```powershell
   gh secret set PARTNER_CENTER_TENANT_ID
   gh secret set PARTNER_CENTER_SELLER_ID
   gh secret set PARTNER_CENTER_CLIENT_ID
   gh secret set PARTNER_CENTER_CLIENT_SECRET
   ```

Until all four exist, the job runs and does nothing.

## First submission

These are the answers for the parts of the first submission that later updates keep.

### Pricing and availability

- Markets: all.
- Visibility: public.
- Pricing: free; no free trial.

### Properties

- Category: **Utilities & tools**; no subcategory.
- Privacy policy URL: `https://nuprint.app/privacy`. Partner Center requires one for this package even though a Store copy sends nothing anywhere except to the printer, and what it keeps (saved printers, job history, the log) stays on the PC.
- Website: `https://nuprint.app`.
- Support contact info: `https://github.com/wapenshaw/nuprint/issues`.
- Product declarations: none apply. NuPrint has no in-app purchases, accounts or ads.
- System requirements: under **Hardware**, mark **Bluetooth** as required.

### Age ratings

Complete the IARC questionnaire as a **utility** app. NuPrint has no violence, sexual content, gambling or controlled substances; users cannot talk to each other or share content through it; it doesn't share the user's location or personal information, and has no purchases. That gives the lowest rating in each region.

### Submission options

- Publishing hold: publish as soon as it passes certification, unless a date is wanted.
- **Notes for certification**:

  > NuPrint prints to small Bluetooth LE thermal printers (the SC03h). Without a printer, a tester can still use everything else: write a note or open a picture or PDF on the left, see the exact paper preview on the right, and save it with "as PNG" above the preview. Settings → Printers → Find printers scans for printers and lists none if there are none nearby. No account or sign-in is needed.

- **Restricted capabilities**: the package declares `runFullTrust`. Justification:

  > NuPrint is a desktop (Win32) application, built with Rust and Tauri, that we package as MSIX for the Store. runFullTrust is required to run it as the same desktop app our standalone installer provides; it is not used to gain any extra access.
  >
  > NuPrint prints notes, pictures and PDF pages to small Bluetooth LE thermal printers. It uses the Windows Bluetooth LE APIs (declared with the bluetooth device capability) to find and talk to the printer, opens documents only when the user picks them in the standard Windows file dialog, and keeps its own data (saved printers, print queue, log) in the package's LocalState folder. It installs no drivers or services, does not change system settings, and does not run at startup or in the background. The Store copy makes no network connections; it does not check for updates itself, because the Store updates it.
  >
  > The source code is public under the MIT licence: https://github.com/wapenshaw/nuprint

### Store listing (English)

**Product name:** NuPrint

**Short description:** Print notes, pictures and PDF pages to a pocket Bluetooth thermal printer from your PC, without the phone app.

**Description:**

> NuPrint prints straight from your Windows PC to small Bluetooth thermal printers, the pocket kind usually driven by a phone app.
>
> Write a note, open a picture or pick one page of a PDF. The preview shows exactly what will come out of the printer, dot for dot, with a millimetre ruler beside it, so you see the paper before you use it. Set the darkness, copies and extra feed, then print.
>
> Jobs go through a queue that keeps them safe: you can cancel one while it is still waiting, print a failed one again, and see a job's history for 30 days. If NuPrint can't tell whether a job printed, it says so and never resends it on its own.
>
> NuPrint has no account, no ads and no tracking. It talks to your printer over Bluetooth and nothing else, and it doesn't need the printer to be paired.
>
> Supported printers: the SC03h (names starting "SC03h-"). Printers with the same settings in the vendor's app are offered as Experimental. NuPrint is free and open source under the MIT licence.

**What's new in this version:** the release's changelog entry.

**Product features** (one per line):

- Exact paper preview with a millimetre ruler
- Notes, pictures and single PDF pages
- Darkness, copies and extra feed for each print
- A print queue that never resends a job on its own
- Save any print as a PNG image
- Light and dark mode, with a choice of accent colours
- No account, ads or tracking

**Search terms:** thermal printer, Bluetooth printer, pocket printer, label printer, mini printer, SC03h, receipt printer

**Screenshots:** at least one desktop screenshot of 1366 × 768 or larger, PNG. Show the note with its preview, a picture, Settings → Printers and dark mode.

**Store logos:** the 9:16 poster art (1440 × 2160), the 1:1 box art (2160 × 2160) and the app tile icon at 300, 150 and 71 pixels. They are the `store-poster`, `store-box` and `tile-*` cards in `site/art/cards.html`; [site/README.md](../site/README.md) explains how to render them.

**Copyright and trademark info:** © 2026 wapenshaw and NuPrint contributors. MIT licence.
