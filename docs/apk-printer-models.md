# Printer models found in the supplied APK

Status: APK-derived inventory with community cross-reference, reviewed 2026-10-13. These are candidate profiles and research notes, not NuPrint compatibility claims. Only lowercase SC03h has a confirmed NuPrint paper result.

## Evidence and scope

Source: `apks/base.apk`, SHA-256 `1BDBD04341EF684557BF1CA2024A0E6C067327E8059F1DC9D1E780FD3CA5C4F1`. Local JADX analysis of `com.Utils.PrintModelUtils`, `com.Utils.PrinterModel.DataBean`, `com.lib.blueUtils.PrinterModelUtils`, `PrintDataUtils`, `BluetoothOrder` and `BLESPPUtils`; see [protocol evidence](protocol.md).

The catalog defines **196 profiles with 187 distinct model strings**. `getData_iPrint()` adds **141 profile entries (140 distinct model strings)**; PR89 appears twice with different profiles. The APK also has separate tinyPrint (122 entries), zhiwuya (117), jzxx (139), CorePrint (8), tattoo, camera and other product lists. These lists overlap and must not be added together as a device count. Profile presence does not establish which names a particular installed app configuration exposes.

`PrinterModelUtils.getModelList()` selects a list using the configured app identity and merges locally stored `DEVINFOJSON` entries ahead of built-in definitions by model number. Therefore this inventory describes static fallback data, not every possible runtime configuration or firmware revision.

The 2026-10-13 extraction decompiled `PrintModelUtils` with JADX `--no-replace-consts`. Earlier output showed some numbers as unrelated library constants (for example `Shape.MASTER_DPI`, which was really 576). Every constructor call was resolved against its matching `DataBean` overload by argument types, so all 196 entries below carry literal values. The extraction scripts and decompiled sources stay in the ignored `.tools/` directory.

Evidence labels in this document:

- **APK-derived:** a constructor value or app behaviour read from the decompiled app.
- **Community-reported:** published by third-party reverse-engineering projects. Useful for planning, but not checked by us; sources sometimes disagree.
- **Hardware-verified:** observed on a NuPrint-owned unit. Today that applies only to SC03h-3DE8; see [validation](validation.md).

## What this means for NuPrint

- **The APK carries per-model print settings, not just names.** Width, resolution, write chunk size, write delay, speed, heat (energy) per darkness level, trailing-feed count and protocol-variant flags are all recorded (see [settings recorded per model](#settings-recorded-per-model)). These are vendor-tuned values, so they are better starting points than guesses or community defaults.
- **35 models have settings identical to the verified SC03h**, and two more differ by one value (see [closest relatives](#closest-relatives-of-the-verified-sc03h)). They are the strongest candidates for the first experimental profiles. Identical app settings still do not prove identical hardware, firmware or Bluetooth layout.
- **Preserve capitalization.** Uppercase SC03H uses image energy 20000, four times SC03h's 5000, and speed 25 instead of 10. The same split exists for X2h/X2H, X5h/X5H, X6h/X6H, X7h/X7H and X103h/X103H: the lowercase variants use 5000 and the uppercase 9500 (X7HP 20000).
- **Several entries are clearly different protocols:**
  - nine models (XW001–XW003, JX001, M01, LY02, PR07, P5AI, AI01) have zero image energy and chunk size;
  - four (JXM800, LY10, LY11, LP100) use the `new-format` framing;
  - twenty mostly wide or label models switch to LZO-compressed image data on newer firmware.

  None of these can reuse the SC03h driver.
- **Wide and label formats** (576 to 2592 dots, 300 dpi heads, label devices) need their own width, media and feed handling.
- **The APK's `useSPP` field is a preference hint**, not proof of Classic-Bluetooth-only support: SC03h sets it but our unit prints over BLE.
- **Do not enable new models by widening the current discovery matcher or copying SC03h values.** Each model gets its own profile and passes the [new-printer acceptance workflow](printer-support.md).

## Settings recorded per model

APK-derived. Field names are the app's own; the meanings come from where the app's print code uses each field.

| Field (APK name) | Table column | Meaning |
| --- | --- | --- |
| `printSize` | Width | Printable dots per row. |
| `devdpi` | DPI | Head resolution: 200 dpi (8 dots/mm) or 300 dpi (12 dots/mm). |
| `imgMTU` | Chunk | Bytes per image write. Zero on models that use another transfer path. |
| `interval` | Delay ms | Delay between BLE writes. NuPrint deliberately paces SC03h more slowly (25 ms between chunks, plus pauses at 4 KiB boundaries) than the app's 4 ms. |
| `imgPrintSpeed` | Speed | Image print-speed parameter sent with BD before printing. |
| `moderationEneragy` (`thinEneragy`, `deepenEneragy`) | Image energy | Heat for normal darkness. The standard path scales it by ±15% per darkness step, the rule NuPrint already applies to SC03h. The light and dark values (shown in brackets when they differ) are used only by the `new-format` path and one "Professional Printer" profile. |
| `textEneragy`, `textPrintSpeed` | — | Text-mode energy and speed. NuPrint rasterizes text as an image, so these are unused. |
| `paperNum` | Feed cmds | Number of 48-step A1 feed commands after the image. SC03h's 2 matches the verified trailing feed. |
| `newFormat` | Flag `new-format` | Every command gets an extra leading `0x12` byte (e.g. status query `12 51 78 A3 …`), and printing uses a different start command. This is a framing variant. |
| `lzoVersion` | Flag `lzo≥…` | Image data is LZO-compressed once the printer reports at least this firmware version (`1` means always). |
| `model == 1` | Flag `wifi` | Wi-Fi/activation flow: the app sends network or activation data during connection. |
| `canPrintLabel`, `labelDevice` | Flags `label`, `label-device` | Label mode, and dedicated label-printer behaviour (gap detection, label back-feed). |
| `grayPrint` | Flag `gray` | Grayscale mode with separate energies. NuPrint prints 1-bit only. |
| `useSPP` | Flag `spp` | App preference for Classic Bluetooth SPP. |
| `hasId` | — | Device-ID handling during discovery. Model `0019B-D` additionally writes a device-ID command on connect. |

## Closest relatives of the verified SC03h

APK-derived. These 35 model strings match SC03h on every protocol-relevant field compared:
- width, resolution and chunk size;
- write delay, speed and energies;
- feed count and paper size;
- the protocol-variant flags (new-format, LZO, Wi-Fi), SPP, grayscale and label flags;
- device-ID handling and MTU negotiation.

`0019B-C`, `0019B-D`, `CLICK-PRINT`, `CLICK-SOUND`, `D1`, `DL_GE225`, `DT1-0`, `DT1-R`, `DTR-R0`, `DY03`, `EMX-040256`, `GB03PL`, `HT0125`, `Luxorp.PX10`, `ML-MP-01`, `MTPR26BK`, `MVMT INK`, `P1`, `Pocket Printer`, `ROSSMANN`, `RT034h`, `SC04h`, `SeznikEcho`, `TCM690464`, `TD-11308`, `UXPORTMIP`, `X100`, `X103h`, `X18`, `X2h`, `X5h`, `X6h`, `X7h`, `XC9`, `XC9-FL01`.

Two more differ by one value: `GB03SL` (write delay 10 ms instead of 4) and `P2` (three trailing feed commands instead of two).

This strongly suggests a shared print head and firmware family sold under many names, but it is an inference. Before any of them is offered:
- their GATT layout must be confirmed. NuPrint's read-only **Check connection** already verifies the AE30 service, AE01 write and AE02/AE04 notifications without printing;
- their status reply must be checked;
- one short strip must be physically confirmed.

`0019B-D` also needs its extra device-ID command reviewed. Generic names such as `Pocket Printer` need more than a name prefix to identify them safely.

## Community-reported observations

These projects reverse-engineered the same printer family independently. Their facts inform planning; their code is not copied.

| Source | What it reports |
| --- | --- |
| [Cat-Printer (NaitLee)](https://github.com/NaitLee/Cat-Printer) `printer_lib/models.py`, CC0 per file header | 15 named models, all 384 dots: GB01, GB02, GB03, GT01, MX05, MX06, MX08, MX09, MX10, MX11, PD01, YT01, SC03h, MXTP and a test model. GB03 is a "new kind": a different start-print command and support for compressed data. MX05/MX06/MX08/MX09/MX10 have a feed-command problem worked around in software. |
| [cataclysm MX10 research](https://github.com/s8n/cataclysm/blob/main/docs/mx10-cat-printer-research.md) | **Same layout as SC03h:** service `AE30` (seen as `AF30` on some macOS stacks), `AE01` write without response, `AE02` notify. Same `51 78 … CRC-8(0x07) FF` framing and the same A2 rows (48 bytes, LSB-first, 1 = black).<br>**Status byte:** paper out, cover open, overheat and low battery in bits 0–3, which agrees with ours. Bit 4 is reported as *pause/buffer full*, where the SC03h APK maps it to *charging*.<br>**MX10 quirks:** no compressed rows (BF), and the A1 feed is unreliable on MX05–MX10 (workaround: blank rows). No status replies mid-job, so it uses fixed pacing (about 200 bytes every 20 ms). A light line about 6 mm in, avoided by leading blank rows. Firmware crashes on back-to-back prints or a low battery.<br>**Energy:** suggested presets about `0x2000`/`0x4000`/`0x6000`. |
| [Cat-Printer](https://github.com/NaitLee/Cat-Printer) README (usage notes, not protocol) | "Windows typically needs longer scan time" (default 4 s) while Android is fine at about 1 s. Agrees with the printer taking about 9 s to reappear after idle on Windows (2026-10-14), which is why the selected-printer lookup window is 30 s. Observed elsewhere, not hardware-verified here. |
| [catprinter (rbaron)](https://github.com/rbaron/catprinter), [issue #2](https://github.com/rbaron/catprinter/issues/2) | GT01, GB02 and GB03 used as example device names. A GT01 negotiated MTU 104 and took 101-byte chunks. Energy is a 16-bit value. A GB02 printed after only the device name was changed from GT01. |
| [CatPrinterBLE (MaikelChan)](https://github.com/MaikelChan/CatPrinterBLE) | MXW01 uses a substantially different protocol from the other cat printers. |
| [MeowPrinter](https://elyez-dev.itch.io/meowprinter) | Lists GB01, GB02, GB03 and MX05. GB03 uses a compressed-bitmap variant. |
| [WerWolv's write-up](https://werwolv.net/blog/cat_printer) | A reverse-engineering walk-through of the same vendor app, with the `51 78` header. |

How the community reports compare with the APK:

- **GB01, GB02 and GT01** are confirmed printing by several projects, and each has its own APK values:
  - GB01: energy 12000 (8000–17500), 63-byte chunks, four feed commands;
  - GB02: energy 12000 (8600–16000), 83-byte chunks, two feeds;
  - GT01: energy 12000, 123-byte chunks.
- **GB03 is contradictory.** Both APK GB03 entries show neither `new-format` nor LZO, yet two projects report compressed or new-kind behaviour. Unit firmware likely differs; treat GB03 as unresolved until a unit is inspected.
- **MX05–MX11, MXW01, PD01, YT01 and MXTP are not in this APK.** They belong to other vendor apps. The MX10 report is still useful: it shows the same Bluetooth layout and framing outside this app, and it shows that feed and pacing quirks vary by model.
- **Status bit 4 differs** between the MX10 report (pause) and the SC03h APK mapping (charging). Status layouts must be confirmed per model, never assumed.

## Shortlist for first experimental profiles

Proposed order. Nothing here is implemented, and each step still needs the [acceptance workflow](printer-support.md).

1. **SC03h-identical models — implemented as experimental (2026-10-13).** 34 profiles in `profiles/` copy the SC03h settings, each with its own exact name prefix and profile ID and no verification entries, so every OS shows them as experimental and saving one needs explicit confirmation. Tests keep them exact copies of the SC03h profile and reject overlapping name prefixes.
   - Folded: `XC9-FL01` shares the `xc9-iprint-384` profile, because its names also start with `XC9-`.
   - Left out: `0019B-D` (the app writes an extra device-ID command on connect, unreviewed) and `Pocket Printer` (a generic name cannot identify the hardware).
   - `GB03SL` and `P2` are included: their one difference (write delay; app trailing-feed count) does not change what NuPrint sends.

   One owner-supplied unit plus **Check connection** and a short strip would move a model to verified basic.
2. **GB01, GB02 and GT01.** They share the framing, and community projects confirm they print. They need APK-specific energy, chunk, delay and feed values, plus a check of their status reply and flow-control messages before printing is offered.
3. **Not yet:**
   - GB03 (conflicting compression evidence);
   - the MX family (outside this APK, with feed and pacing quirks);
   - zero-energy, `new-format`, LZO and Wi-Fi models;
   - label devices and wide formats.

   Each needs protocol work or new evidence first.

## Full static catalog

Independently extracted constructor values, not vendor implementation code.
- **iPrint** marks membership in `getData_iPrint()`, not hardware validation.
- **Image energy** shows the normal value, with the light–dark range when it differs.
- **≡SC03h** marks entries identical to SC03h on the compared fields.
- Model strings and prefixes preserve case and whitespace.
- **Line** refers to the local `--no-replace-consts` JADX output of `PrintModelUtils.java` and may shift with another decompiler version.

| Model | Name prefix | Width | DPI | Chunk | Delay ms | Speed | Image energy | Feed cmds | Flags | iPrint | Line |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `XW001` | `PR20-` | 384 | 200 | 0 | 2 | 45 | 0 | 2 | — | yes | 12 |
| `XW002` | `PR30-` | 576 | 300 | 0 | 2 | 45 | 0 | 2 | — | — | 13 |
| `XW003` | `PR25-` | 384 | 200 | 0 | 5 | 45 | 0 | 2 | wifi | yes | 14 |
| `JX001` | `JX01-` | 384 | 200 | 0 | 3 | 90 | 0 | 2 | — | yes | 15 |
| `JXM800` | `GG-D2100-` | 1728 | 200 | 183 | 4 | 26 | 10 (5–12) | 2 | new-format spp | yes | 16 |
| `M01` | `M01-` | 384 | 200 | 0 | 5 | 45 | 0 | 2 | wifi | yes | 17 |
| `GB01` | `GB01-` | 384 | 200 | 63 | 6 | 35 | 12000 (8000–17500) | 4 | — | yes | 18 |
| `GB02` | `GB02-` | 384 | 200 | 83 | 6 | 26 | 12000 (8600–16000) | 2 | — | yes | 19 |
| `GB03` | `GB03-` | 384 | 200 | 83 | 6 | 10 | 9000 | 2 | label | — | 20 |
| `GB03` | `GB03-` | 384 | 200 | 83 | 6 | 30 | 7500 | 2 | label | yes | 21 |
| `CMT-0510` | `CMT-0510-` | 384 | 200 | 83 | 6 | 30 | 7500 | 2 | label | yes | 22 |
| `GV-MA211` | `GV-MA211-` | 384 | 200 | 83 | 6 | 30 | 7500 | 2 | label | yes | 23 |
| `SC03` | `SC03-` | 384 | 200 | 83 | 6 | 30 | 7500 | 2 | label | yes | 24 |
| `FL01` | `FL01-` | 384 | 200 | 183 | 4 | 20 | 7500 | 2 | label gray spp | yes | 25 |
| `KF-5` | `KF-5-` | 384 | 200 | 183 | 4 | 20 | 7500 | 2 | label gray spp | yes | 26 |
| `SC04` | `SC04-` | 384 | 200 | 83 | 6 | 30 | 7500 | 2 | label | yes | 27 |
| `GB04` | `GB04-` | 384 | 200 | 83 | 6 | 26 | 12000 (8600–16000) | 4 | — | yes | 28 |
| `GB05` | `GB05-` | 384 | 200 | 123 | 6 | 26 | 10000 | 4 | — | yes | 29 |
| `GB06` | `GB06-` | 384 | 200 | 83 | 6 | 10 | 9000 | 2 | label | — | 30 |
| `GB06` | `GB06-` | 384 | 200 | 83 | 6 | 30 | 7500 | 2 | label | yes | 31 |
| `GT01` | `GT01-` | 384 | 200 | 123 | 4 | 30 | 12000 | 2 | — | yes | 32 |
| `GT02` | `GT02-` | 384 | 200 | 123 | 4 | 30 | 9000 | 2 | label | yes | 33 |
| `58P5` | `58P5-` | 384 | 200 | 123 | 6 | 10 | 6500 | 2 | — | yes | 34 |
| `WL01` | `WL01-` | 384 | 200 | 123 | 6 | 10 | 6500 | 2 | — | yes | 35 |
| `X5` | `X5-` | 384 | 200 | 123 | 6 | 10 | 7500 | 2 | label | — | 36 |
| `X5` | `X5-` | 384 | 200 | 123 | 6 | 30 | 7500 | 2 | label | yes | 37 |
| `X7` | `X7-` | 384 | 200 | 123 | 6 | 10 | 7500 | 2 | label | — | 38 |
| `X7` | `X7-` | 384 | 200 | 123 | 6 | 30 | 7500 | 2 | label | yes | 39 |
| `S102` | `S102-` | 384 | 200 | 123 | 6 | 10 | 6500 | 2 | — | — | 40 |
| `S102` | `S102-` | 384 | 200 | 123 | 6 | 30 | 7500 | 2 | — | yes | 41 |
| `LY02` | `LY02-` | 384 | 200 | 0 | 2 | 45 | 0 | 2 | — | yes | 42 |
| `LY05` | `LY05-` | 576 | 200 | 153 | 6 | 40 | 12000 | 2 | — | yes | 43 |
| `X1` | `X1-` | 576 | 200 | 183 | 6 | 40 | 8500 | 2 | — | yes | 44 |
| `HD1` | `HD1-` | 576 | 200 | 183 | 6 | 40 | 8500 | 2 | — | — | 45 |
| `PR30` | `PR30-` | 576 | 200 | 183 | 4 | 18 | 10 (5–12) | 2 | — | yes | 46 |
| `PR02` | `PR02-` | 384 | 200 | 123 | 6 | 10 | 5500 | 2 | label | yes | 47 |
| `P10` | `P10-` | 384 | 200 | 123 | 6 | 10 | 5500 | 2 | — | yes | 48 |
| `P7` | `P7-` | 384 | 200 | 183 | 4 | 30 | 7500 | 2 | label spp | yes | 49 |
| `X6` | `X6-` | 384 | 200 | 123 | 4 | 10 | 9000 | 2 | label | — | 50 |
| `X6` | `X6-` | 384 | 200 | 123 | 6 | 30 | 7500 | 2 | label | yes | 51 |
| `DY01` | `DY01-` | 384 | 200 | 123 | 6 | 30 | 7500 | 2 | label | yes | 52 |
| `IprintIt` | `IprintIt Printer-` | 384 | 200 | 123 | 6 | 30 | 7500 | 2 | label | yes | 53 |
| `LP6` | `LP6-` | 384 | 200 | 123 | 6 | 30 | 7500 | 2 | label | yes | 54 |
| `PR35` | `PR35-` | 576 | 200 | 183 | 6 | 18 | 12000 | 2 | wifi | yes | 55 |
| `S01` | `S01-` | 384 | 200 | 123 | 6 | 10 | 6500 | 2 | wifi | yes | 56 |
| `LT01` | `LT01-` | 384 | 200 | 83 | 6 | 10 | 4500 | 2 | label | yes | 57 |
| `GT08` | `GT08-` | 1728 | 200 | 512 | 4 | 30 | 3500 (3000–4000) | 2 | spp | yes | 58 |
| `GW08` | `GW08-` | 1728 | 200 | 512 | 4 | 30 | 3500 (3000–4000) | 2 | spp | yes | 59 |
| `GW09` | `GW09-` | 1728 | 200 | 512 | 4 | 30 | 12000 | 2 | spp | yes | 60 |
| `PR88` | `PR88-` | 1728 | 200 | 512 | 4 | 30 | 3500 (3000–4000) | 2 | spp | yes | 61 |
| `PR89` | `PR89-` | 1728 | 200 | 512 | 4 | 40 | 27000 | 2 | spp | yes | 62 |
| `PR89` | `PR89-` | 1728 | 200 | 512 | 4 | 40 | 27000 | 2 | lzo≥2.0.66 spp | yes | 63 |
| `X8-L` | `X8-L-` | 1728 | 200 | 512 | 4 | 40 | 27000 | 3 | lzo≥2.0.66 spp | yes | 64 |
| `X8-W` | `X8-W-` | 1728 | 200 | 512 | 4 | 40 | 27000 | 3 | lzo≥2.0.66 wifi spp | yes | 65 |
| `ZP801` | `ZP801-` | 1728 | 200 | 512 | 4 | 40 | 27000 | 2 | lzo≥2.0.66 spp | yes | 66 |
| `PR893` | `PR893-` | 2592 | 300 | 512 | 4 | 65 | 8000 | 2 | lzo≥1 spp | yes | 67 |
| `A43` | `A43-` | 1728 | 200 | 300 | 4 | 40 | 30000 (17000–30000) | 2 | lzo≥2.0.67Z spp | yes | 68 |
| `A4300` | `A4300-` | 2496 | 300 | 512 | 2 | 40 | 10000 | 2 | lzo≥2.0.67Z spp | yes | 69 |
| `MPA81` | `MPA81-` | 1728 | 200 | 512 | 2 | 40 | 30000 (17000–30000) | 2 | lzo≥1 spp | yes | 70 |
| `P4` | `P4-` | 1728 | 200 | 512 | 4 | 40 | 17000 | 2 | spp | yes | 71 |
| `X9` | `X9-` | 1728 | 200 | 512 | 4 | 40 | 15000 | 3 | lzo≥2.1.1 spp | yes | 72 |
| `MV-B530` | `MV-B530-` | 1728 | 200 | 512 | 4 | 40 | 15000 | 3 | lzo≥0 spp | yes | 73 |
| `QDID` | `QDID-` | 1728 | 200 | 512 | 4 | 40 | 15000 | 3 | lzo≥2.1.1 spp | — | 74 |
| `GL-VS9` | `GL-VS9-` | 1728 | 200 | 512 | 4 | 40 | 15000 | 3 | lzo≥2.1.1 spp | yes | 75 |
| `ZP802` | `ZP802-` | 2496 | 300 | 512 | 4 | 20 | 8000 | 2 | lzo≥1.0.3 spp | yes | 76 |
| `ZPA4Z1` | `ZPA4Z1-` | 1728 | 200 | 512 | 4 | 40 | 27000 | 2 | spp | yes | 77 |
| `A42II` | `A42II-` | 1664 | 200 | 183 | 4 | 20 | 12000 (10500–12500) | 2 | spp | yes | 78 |
| `A41II` | `A41II-` | 1664 | 200 | 183 | 4 | 20 | 12000 (10500–12500) | 2 | spp | yes | 79 |
| `A41III` | `A41III-` | 1664 | 300 | 183 | 4 | 20 | 12000 (10500–12500) | 2 | spp | yes | 80 |
| `X8` | `X8-` | 1728 | 200 | 183 | 4 | 30 | 3500 (3000–4000) | 2 | wifi spp | yes | 81 |
| `S101` | `S101-` | 384 | 200 | 123 | 6 | 10 | 6500 | 2 | — | — | 82 |
| `S101` | `S101-` | 384 | 200 | 123 | 6 | 30 | 7500 | 2 | — | yes | 83 |
| `PR07` | `PR07-` | 384 | 200 | 0 | 5 | 45 | 0 | 2 | wifi | yes | 84 |
| `LY01` | `LY01-` | 384 | 200 | 0 | 4 | 40 | 9500 (0–20000) | 2 | — | yes | 85 |
| `LY03` | `LY03-` | 384 | 200 | 83 | 6 | 10 | 7500 | 2 | — | yes | 86 |
| `GT03` | `GT03-` | 384 | 200 | 123 | 4 | 30 | 12000 | 2 | — | yes | 87 |
| `GT04` | `GT04-` | 384 | 200 | 123 | 4 | 30 | 8000 | 2 | — | yes | 88 |
| `P5AI` | `P5AI-` | 384 | 200 | 0 | 5 | 45 | 0 | 2 | wifi | yes | 89 |
| `LY10` | `LY10-` | 384 | 200 | 183 | 4 | 26 | 12 (8–15) | 2 | new-format spp | yes | 90 |
| `LY11` | `LY11-` | 576 | 300 | 183 | 4 | 26 | 3 (0–15) | 3 | new-format spp | yes | 91 |
| `LP100` | `LY10-` | 384 | 200 | 183 | 4 | 26 | 100 | 2 | new-format spp | — | 92 |
| `AI01` | `AI01-` | 384 | 200 | 0 | 5 | 45 | 0 | 2 | wifi | yes | 93 |
| `M2` | `M2-` | 384 | 200 | 83 | 4 | 10 | 4500 | 2 | label | yes | 94 |
| `ZHHC` | `ZHHC-` | 384 | 200 | 123 | 4 | 10 | 9000 | 2 | — | yes | 95 |
| `P6` | `P6-` | 384 | 200 | 180 | 4 | 10 | 9500 | 2 | label spp | yes | 96 |
| `P7H` | `P7H-` | 384 | 200 | 180 | 4 | 10 | 9500 | 2 | label spp | yes | 97 |
| `X2H` | `X2H-` | 384 | 200 | 180 | 4 | 10 | 9500 | 2 | label gray spp | yes | 98 |
| `X102` | `X102-` | 384 | 200 | 180 | 4 | 10 | 9500 | 2 | label gray spp | — | 99 |
| `X6HP` | `X6HP-` | 384 | 200 | 180 | 4 | 10 | 9500 | 2 | label gray spp | yes | 100 |
| `X5HP` | `X5HP-` | 384 | 200 | 180 | 4 | 10 | 9500 | 2 | label gray spp | yes | 101 |
| `X7HP` | `X7HP-` | 384 | 200 | 180 | 4 | 10 | 20000 | 2 | label gray spp | yes | 102 |
| `ewtto ET-Z0504` | `ewtto ET-Z0504-` | 384 | 200 | 180 | 4 | 10 | 9500 | 2 | label gray spp | yes | 103 |
| `X103H` | `X103H-` | 384 | 200 | 180 | 4 | 10 | 9500 | 2 | label gray spp | yes | 104 |
| `X103h` | `X103h-` | 384 | 200 | 180 | 4 | 10 | 5000 | 2 | **≡SC03h** label gray spp | yes | 105 |
| `X6H` | `X6H-` | 384 | 200 | 180 | 4 | 10 | 9500 | 2 | label spp | yes | 106 |
| `X5H` | `X5H-` | 384 | 200 | 180 | 4 | 10 | 9500 | 2 | label gray spp | yes | 107 |
| `AN01` | `AN01-` | 384 | 200 | 103 | 4 | 10 | 5000 (4000–6000) | 2 | gray | yes | 108 |
| `XiaoWa` | `XiaoWa-` | 384 | 200 | 183 | 4 | 20 | 12000 | 2 | gray | — | 109 |
| `CP01` | `CP01-` | 384 | 200 | 183 | 4 | 20 | 12000 | 2 | gray | yes | 110 |
| `S5A` | `S5A-` | 384 | 200 | 183 | 4 | 20 | 12000 | 2 | wifi gray | yes | 111 |
| `P20 max` | `P20 max-` | 384 | 200 | 183 | 4 | 20 | 12000 | 2 | wifi gray | yes | 112 |
| `S9A` | `S9A-` | 384 | 200 | 183 | 4 | 20 | 12000 | 2 | gray | yes | 113 |
| `DY33A` | `DY33A-` | 384 | 200 | 183 | 4 | 20 | 12000 | 2 | gray | yes | 114 |
| `YMS-BT01` | `YMS-BT01-` | 384 | 200 | 183 | 4 | 20 | 5000 | 2 | gray | yes | 115 |
| `WJ-HOT-PRT` | `WJ-HOT-PRT-` | 384 | 200 | 183 | 4 | 20 | 12000 | 2 | gray | yes | 116 |
| `SeznikNeo` | `SeznikNeo-` | 384 | 200 | 183 | 4 | 20 | 12000 | 2 | gray | — | 117 |
| `JRX01` | `JRX01-` | 384 | 200 | 183 | 4 | 20 | 12000 | 2 | gray | yes | 118 |
| `RS9000` | `RS9000-` | 384 | 200 | 183 | 4 | 20 | 12000 | 2 | gray | yes | 119 |
| `DY49` | `DY49-` | 384 | 200 | 183 | 4 | 20 | 12000 | 2 | gray | — | 120 |
| `QDX01` | `QDX01-` | 384 | 200 | 183 | 4 | 20 | 12000 | 2 | gray | yes | 121 |
| `wts07` | `wts07-` | 384 | 200 | 183 | 4 | 20 | 12000 | 2 | gray | yes | 122 |
| `GT10` | `GT10-` | 1728 | 200 | 383 | 4 | 30 | 3500 (3000–4000) | 2 | lzo≥1 spp | yes | 123 |
| `GT09` | `GT09-` | 1728 | 200 | 383 | 4 | 30 | 3500 (3000–4000) | 2 | spp | yes | 124 |
| `A200` | `A200-` | 1664 | 200 | 183 | 4 | 20 | 12000 (10500–12500) | 2 | spp | yes | 125 |
| `X7H` | `X7H-` | 384 | 200 | 180 | 4 | 10 | 9500 | 2 | label gray spp | yes | 126 |
| `X2h` | `X2h-` | 384 | 200 | 180 | 4 | 10 | 5000 | 2 | **≡SC03h** label gray spp | yes | 127 |
| `X5h` | `X5h-` | 384 | 200 | 180 | 4 | 10 | 5000 | 2 | **≡SC03h** label gray spp | yes | 128 |
| `MTPR26BK` | `MTPR26BK-` | 384 | 200 | 180 | 4 | 10 | 5000 | 2 | **≡SC03h** label gray spp | — | 129 |
| `ML-MP-01` | `ML-MP-01-` | 384 | 200 | 180 | 4 | 10 | 5000 | 2 | **≡SC03h** label gray spp | yes | 130 |
| `Luxorp.PX10` | `Luxorp.PX10-` | 384 | 200 | 180 | 4 | 10 | 5000 | 2 | **≡SC03h** label gray spp | — | 131 |
| `DTR-R0` | `DTR-R0-` | 384 | 200 | 180 | 4 | 10 | 5000 | 2 | **≡SC03h** label gray spp | — | 132 |
| `ROSSMANN` | `ROSSMANN-` | 384 | 200 | 180 | 4 | 10 | 5000 | 2 | **≡SC03h** label gray spp | yes | 133 |
| `X18` | `X18-` | 384 | 200 | 180 | 4 | 10 | 5000 | 2 | **≡SC03h** label gray spp | — | 134 |
| `CLICK-SOUND` | `CLICK-SOUND-` | 384 | 200 | 180 | 4 | 10 | 5000 | 2 | **≡SC03h** label gray spp | — | 135 |
| `CLICK-PRINT` | `CLICK-PRINT-` | 384 | 200 | 180 | 4 | 10 | 5000 | 2 | **≡SC03h** label gray spp | — | 136 |
| `DT1-R` | `DT1-R-` | 384 | 200 | 180 | 4 | 10 | 5000 | 2 | **≡SC03h** label gray spp | — | 137 |
| `MVMT INK` | `MVMT INK-` | 384 | 200 | 180 | 4 | 10 | 5000 | 2 | **≡SC03h** label gray spp | — | 138 |
| `0019B-D` | `0019B-D-` | 384 | 200 | 180 | 4 | 10 | 5000 | 2 | **≡SC03h** label gray spp | yes | 139 |
| `EMX-040256` | `EMX-040256-` | 384 | 200 | 180 | 4 | 10 | 5000 | 2 | **≡SC03h** label gray spp | — | 140 |
| `HT0125` | `HT0125-` | 384 | 200 | 180 | 4 | 10 | 5000 | 2 | **≡SC03h** label gray spp | — | 141 |
| `DT1-0` | `DT1-0-` | 384 | 200 | 180 | 4 | 10 | 5000 | 2 | **≡SC03h** label gray spp | — | 142 |
| `X7h` | `X7h-` | 384 | 200 | 180 | 4 | 10 | 5000 | 2 | **≡SC03h** label gray spp | yes | 143 |
| `0019B-C` | `0019B-C-` | 384 | 200 | 180 | 4 | 10 | 5000 | 2 | **≡SC03h** label gray spp | yes | 144 |
| `X100` | `X100-` | 384 | 200 | 180 | 4 | 10 | 5000 | 2 | **≡SC03h** label gray spp | — | 145 |
| ` X101H` | ` X101H-` | 384 | 200 | 180 | 10 | 10 | 9500 | 2 | label gray spp | — | 146 |
| `LGM01` | `LGM01-` | 384 | 200 | 180 | 4 | 10 | 20000 | 2 | label gray spp | yes | 147 |
| `P1` | `P1-` | 384 | 200 | 180 | 4 | 10 | 5000 | 2 | **≡SC03h** label gray spp | yes | 148 |
| `DY03` | `DY03-` | 384 | 200 | 180 | 4 | 10 | 5000 | 2 | **≡SC03h** label gray spp | yes | 149 |
| `SC03h` | `SC03h-` | 384 | 200 | 180 | 4 | 10 | 5000 | 2 | label gray spp | yes | 150 |
| `TD-11308` | `TD-11308-` | 384 | 200 | 180 | 4 | 10 | 5000 | 2 | **≡SC03h** label gray spp | yes | 151 |
| `P5` | `P5-` | 576 | 300 | 280 | 4 | 10 | 5000 | 2 | label gray spp | — | 152 |
| `XC9` | `XC9-` | 384 | 200 | 180 | 4 | 10 | 5000 | 2 | **≡SC03h** label gray spp | — | 153 |
| `XC9-FL01` | `XC9-FL01-` | 384 | 200 | 180 | 4 | 10 | 5000 | 2 | **≡SC03h** label gray spp | — | 154 |
| `PT001` | `PT001-` | 384 | 200 | 180 | 4 | 10 | 20000 | 2 | label gray spp | yes | 155 |
| `SC04h` | `SC04h-` | 384 | 200 | 180 | 4 | 10 | 5000 | 2 | **≡SC03h** label gray spp | yes | 156 |
| `P2` | `P2-` | 384 | 200 | 180 | 4 | 10 | 5000 | 3 | label gray spp | yes | 157 |
| `SC05` | `SC05-` | 384 | 200 | 180 | 4 | 10 | 9500 | 2 | label gray spp | yes | 158 |
| `IM.04` | `IM.04-` | 384 | 200 | 180 | 4 | 10 | 9500 | 2 | label gray spp | yes | 159 |
| `RT034h` | `RT034h-` | 384 | 200 | 180 | 4 | 10 | 5000 | 2 | **≡SC03h** label gray spp | yes | 160 |
| `D1` | `D1-` | 384 | 200 | 180 | 4 | 10 | 5000 | 2 | **≡SC03h** label gray spp | yes | 161 |
| `UXPORTMIP` | `UXPORTMIP-` | 384 | 200 | 180 | 4 | 10 | 5000 | 2 | **≡SC03h** label gray spp | yes | 162 |
| `GB03SL` | `GB03SL-` | 384 | 200 | 180 | 10 | 10 | 5000 | 2 | label gray spp | yes | 163 |
| `GB03SH` | `GB03SH-` | 384 | 200 | 180 | 4 | 25 | 9500 | 2 | label gray spp | yes | 164 |
| `FC02` | `FC02-` | 384 | 200 | 180 | 4 | 25 | 20000 | 2 | label gray spp | — | 165 |
| `A33` | `A33-` | 384 | 200 | 183 | 4 | 25 | 9500 | 2 | label gray spp | yes | 166 |
| `SC03H` | `SC03H-` | 384 | 200 | 180 | 4 | 25 | 20000 | 2 | label gray spp | yes | 167 |
| `GB03PL` | `GB03PL-` | 384 | 200 | 180 | 4 | 10 | 5000 | 2 | **≡SC03h** label gray spp | yes | 168 |
| `GB03PH` | `GB03PH-` | 384 | 200 | 180 | 4 | 25 | 9500 | 2 | label gray spp | yes | 169 |
| `GB02SH` | `GB02SH-` | 384 | 200 | 180 | 4 | 25 | 7000 | 2 | label gray spp | yes | 170 |
| `Pocket Printer` | `Pocket Printer-` | 384 | 200 | 180 | 4 | 10 | 5000 | 2 | **≡SC03h** label gray spp | — | 171 |
| `TCM690464` | `TCM690464-` | 384 | 200 | 180 | 4 | 10 | 5000 | 2 | **≡SC03h** label gray spp | — | 172 |
| `15P3` | `15P3-` | 96 | 200 | 180 | 4 | 10 | 5000 | 2 | label-device spp | yes | 173 |
| `YK06` | `YK06-` | 96 | 200 | 180 | 4 | 10 | 5000 | 2 | label-device spp | yes | 174 |
| `WQ02` | `WQ02-` | 96 | 200 | 180 | 4 | 10 | 5000 | 2 | label-device spp | yes | 175 |
| `L1` | `L1-` | 96 | 200 | 180 | 4 | 8 | 1500 | 2 | label-device spp | — | 176 |
| `U2` | `U2-` | 96 | 200 | 180 | 4 | 8 | 1500 | 2 | label-device spp | — | 177 |
| `X16` | `X16-` | 384 | 200 | 180 | 4 | 10 | 5000 | 3 | wifi gray spp | yes | 178 |
| `Audio Print` | `Audio Print-` | 384 | 200 | 180 | 4 | 10 | 5000 | 3 | wifi gray spp | yes | 179 |
| `TRAiPrinter` | `TRAiPrinter-` | 384 | 200 | 180 | 4 | 10 | 5000 | 3 | wifi gray spp | — | 180 |
| `Totori` | `Totori-` | 384 | 200 | 180 | 4 | 10 | 5000 | 3 | wifi gray spp | — | 181 |
| `VOICE PRINT` | `VOICE PRINT-` | 384 | 200 | 180 | 4 | 10 | 5000 | 3 | wifi gray spp | — | 182 |
| `KIDYPRINTER` | `KIDYPRINTER-` | 384 | 200 | 180 | 4 | 10 | 5000 | 3 | wifi gray spp | — | 183 |
| `LISCIANI` | `LISCIANI-` | 384 | 200 | 180 | 4 | 10 | 5000 | 3 | wifi gray spp | — | 184 |
| `AiPrint` | `AiPrint-` | 384 | 200 | 180 | 4 | 10 | 5000 | 3 | wifi gray spp | yes | 185 |
| `AiPrintT1` | `AiPrintT1-` | 384 | 200 | 180 | 4 | 10 | 10000 | 3 | wifi gray spp | — | 186 |
| `A2` | `A2-` | 384 | 200 | 180 | 4 | 10 | 5000 | 3 | wifi gray spp | yes | 187 |
| `A3` | `A3-` | 384 | 200 | 180 | 4 | 10 | 5000 | 3 | wifi gray spp | yes | 188 |
| `X6h` | `X6h-` | 384 | 200 | 180 | 4 | 10 | 5000 | 2 | **≡SC03h** label gray spp | yes | 189 |
| `U1` | `U1-` | 384 | 200 | 180 | 4 | 10 | 20000 | 2 | label-device gray spp | yes | 190 |
| `EA-LBLU1-WH` | `EA-LBLU1-WH-` | 384 | 200 | 180 | 4 | 10 | 20000 | 2 | label-device gray spp | — | 191 |
| `D100` | `D100-` | 864 | 200 | 180 | 4 | 10 | 5000 | 2 | label-device gray spp | — | 192 |
| `Mini Printer` | `CTP500` | 384 | 200 | 180 | 4 | 20 | 14000 | 2 | label gray spp | — | 193 |
| `Label Printer` | `CPLM10` | 96 | 200 | 180 | 4 | 10 | 1400 | 2 | label-device spp | yes | 194 |
| `Shipping Printer` | `CTP800BD` | 800 | 200 | 180 | 4 | 10 | 8000 | 2 | lzo≥1 label-device gray spp | — | 195 |
| `Shipping Printer` | `CTP750BY` | 800 | 200 | 180 | 4 | 10 | 8000 | 2 | lzo≥1 label-device gray spp | — | 196 |
| `Professional Printer` | `CTP100LG` | 1728 | 200 | 512 | 4 | 20 | 12000 | 5 | lzo≥1 spp | — | 197 |
| `PR08` | `PR08-` | 1728 | 200 | 512 | 4 | 20 | 12000 | 5 | lzo≥1 spp | yes | 198 |
| `S600` | `S600-` | 800 | 200 | 180 | 4 | 10 | 8000 | 2 | lzo≥1 label-device gray spp | yes | 199 |
| `KS-1` | `KS-1-` | 800 | 200 | 180 | 4 | 10 | 8000 | 2 | lzo≥1 label-device gray spp | yes | 200 |
| `DL_X7` | `DL_X7-` | 864 | 200 | 512 | 4 | 10 | 7000 | 2 | label-device gray spp | — | 201 |
| `DL_X7Pro` | `DL_X7Pro-` | 1280 | 300 | 512 | 4 | 10 | 7000 | 2 | label-device gray spp | — | 202 |
| `DL_X2` | `DL_X2-` | 384 | 200 | 180 | 4 | 10 | 8000 | 2 | label gray spp | — | 203 |
| `DL_GE225` | `DL_GE225-` | 384 | 200 | 180 | 4 | 10 | 5000 | 2 | **≡SC03h** label gray spp | — | 204 |
| `DL_X2Pro` | `DL_X2Pro-` | 576 | 300 | 280 | 4 | 10 | 5000 | 2 | label gray spp | — | 205 |
| `SeznikEcho` | `SeznikEcho-` | 384 | 200 | 180 | 4 | 10 | 5000 | 2 | **≡SC03h** label gray spp | — | 206 |
| `CameraPrint` | `CameraPrint-` | 384 | 200 | 180 | 4 | 10 | 5000 | 3 | wifi gray spp | — | 207 |
