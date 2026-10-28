# Supporting multiple Bluetooth printers

Status: implemented for one protocol (`iprint-5178`) with one verified profile (SC03h) and 34 experimental profiles whose vendor-app settings are identical to it (see [the APK catalog](apk-printer-models.md#closest-relatives-of-the-verified-sc03h)); the rest is the proposed contract. Only SC03h has physical evidence. No new model is supported merely because this document names a possible protocol family.

The [supplied APK catalog](apk-printer-models.md) records 196 static profiles / 187 distinct model strings, their name prefixes, dimensions and iPrint membership. Use it to select and research candidate hardware, not as a ready-to-enable support list.

## Extension model

Keep transport, protocol and model profile separate. Two models may share a codec but differ in heat or width. Two devices may share a BLE UUID and speak incompatible protocols. A Classic/SPP device cannot be handled by the BLE backend merely by changing a UUID.

| Interface | Owns | Must not own |
| --- | --- | --- |
| `Transport` | Discovery/identity, connection, endpoint reads/writes, notification delivery, write limits, cleanup | Raster meaning, heat defaults, status-bit interpretation |
| `PrinterProtocol` | Framing/CRC, handshake/query plan, job encoding, parsing, flow-control semantics, completion evidence | Desktop UI, database, OS device enumeration |
| `ModelProfile` | Exact matching rules, endpoint bindings, capabilities, dimensions, tested settings and evidence | Arbitrary executable code or guessed universal defaults |
| `PrinterSession` | Selected profile + protocol + transport, ordered exchange and bounded lifecycle | Concurrent jobs sharing the same connection |

Protocols return an ordered exchange plan containing typed writes, notification waits, supported status queries and bounded delays. Preserve command/response boundaries where the model requires them; do not assume every printer accepts one concatenated byte stream. Common session code applies cancellation and disconnect semantics without hardcoding SC03h's AE flow messages.

## Profile schema

Use reviewed, versioned JSON profiles bundled with the application, validated before device use. Start with built-in Rust protocol adapters; do not add downloadable native plugins or executable scripting in profiles. Introducing a new protocol requires reviewed code and a release. Adding a profile may reuse a driver only when evidence confirms compatibility.

Required field groups:

| Group | Fields / rules |
| --- | --- |
| Identity | Stable profile ID, schema version, profile revision, display/vendor/model, protocol ID/version |
| Match | Case-sensitive names/prefixes; optional manufacturer/service data; required connected endpoints/properties; firmware constraints when known |
| Transport | `ble_gatt` initially; endpoint roles, exact UUIDs, required/optional notify, write mode, tested write ceiling and pacing |
| Raster | Printable width, horizontal/vertical dots-per-mm, supported height bounds, pixel format, row alignment/padding and driver packing variant |
| Media | Continuous/fixed-length/die-cut capabilities; trailing-feed support, unit conversion, default/min/max/increment, zero-feed behavior and calibration evidence; supported margins/calibration only if known |
| Settings | Named density/speed presets mapped by the driver; no unbounded heat control |
| Status | Supported fields, whether fresh status is required, parser variant, unknown-value handling |
| Lifecycle | Query/handshake variant, buffer semantics, cancellation limits, completion policy and deadlines |
| Evidence | APK/manual/source references, firmware/OS/adapter observations, synthetic fixtures and physical result dates |

Example registry identity: `sc03h-iprint-384`, protocol `iprint-5178`, transport `ble_gatt`. These are proposed internal IDs. The initial profile copies the verified lowercase model's settings from [protocol.md](protocol.md): 384 dots, LSB-first black bits, 5000 normal energy, speed 10, 180-byte maximum model chunk and AF30-advertisement/AE30-service behavior. Uppercase SC03H receives a separate profile only after validation.

Profile revisions are immutable within an admitted job. Persist ID, revision, protocol version and effective settings in each job snapshot. An update cannot change the meaning of queued bytes; hold and ask the user to rerender if the old implementation can no longer be executed safely. No automatic migration to an unrelated profile.

## Matching and unknown devices

1. Scan and show supported candidates; provide a separate diagnostic view for unknown nearby devices when requested.
2. Evaluate discovery hints locally. A name or advertised UUID yields a candidate, not permission to print.
3. On selection, enumerate GATT services/properties without speculative vendor writes.
4. Resolve candidate profiles using exact identity rules and connected endpoint requirements. Where multiple profiles remain, present the ambiguity and disable printing.
5. Issue only the selected profile's known non-printing query sequence. A3/A8 are SC03h-family behavior, not generic Bluetooth discovery.
6. Require the profile's readiness/handshake criteria, then admit printing with its capabilities.

A manual profile choice cannot bypass endpoint compatibility or convert an unsupported printer into a verified one. If a model lacks readable identity, mark that limitation and require a deliberately selected experimental profile with a documented physical test. Do not probe unknown devices by cycling ESC/POS, SC03h and other print/reset/calibration commands.

Persist an app-assigned printer ID with OS/backend-scoped identity, observed name and profile association. Do not use a globally assumed MAC address as the cross-platform primary key. Rediscovery can propose a reassociation but must not route pending jobs to a similarly named device silently.

## Capability-driven UI and rendering

The selected profile supplies paper choices, width, resolution, permitted darkness/speed settings, media/feed behavior and readable status fields. A profile switch invalidates the current raster snapshot and rerenders at the new dimensions. Retain the source design rather than scaling already-binarized pixels between printers.

Common bitmap convention: row-major, 1=black, MSB-first within canonical storage bytes, row stride `ceil(width/8)`, zero padding outside printable width. Drivers transform into their required bit/byte order. SC03h's wire encoding remains **LSB-first**. The canonical convention is a new Rust internal contract, not a change to the physical protocol. Include both-width and non-byte-aligned fixtures to catch accidental SC03h-only assumptions.

Resolve text dimensions in physical units using each model's horizontal/vertical resolution. Blank padding, feed amount, margins, compression and grey levels belong to the selected driver/profile. Do not send a 384-dot page blindly to a 203-, 576- or other-width printer.

Completion policies:

- **Status-confirmed ready:** when the driver has a verified readiness response after settling/draining, show transfer complete plus printer-ready evidence.
- **Acknowledged acceptance:** where a model acknowledges a job but not paper completion, label it accepted and retain the distinction.
- **Unconfirmed transfer:** for write-only models, apply a tested bounded drain policy and show bytes sent/output unconfirmed. Do not invent status support.

All models preserve `Uncertain` after a partial transfer failure. Models without a reliable abort command can stop further sending only; the UI explains that buffered data may continue printing.

## Add-a-printer workflow and support levels

1. Record exact vendor/model/name casing, firmware, connection type and service inventory. Obtain a manual, protocol reference, app analysis or authorized capture.
2. Determine whether an existing protocol implementation actually matches; otherwise add a new adapter behind the same interface.
3. Define bounded model presets and capabilities with their evidence. Unknown settings remain unavailable.
4. Add sanitized frame/reply fixtures and offline tests for matching, packing, CRC/lengths, status, flow and failure handling.
5. Run one authorized short synthetic strip, then relevant rendering/length/error cases. Record paper observations separately.
6. Test each claimed OS/backend; publish the exact supported matrix and known limits.

Track support per **model/profile revision × firmware × OS/backend**, not one global Boolean:

| Level | Meaning |
| --- | --- |
| Candidate | Discovered/researched; no printing offered in normal UI |
| Experimental | Implemented with offline tests; explicit opt-in and stated missing evidence |
| Verified basic | Packaged connection/status as supported and short physical print confirmed on this matrix entry |
| Release supported | Relevant length, recovery, media and packaging acceptance passed for declared features |

## Settings edited on this computer

An owner can tune a saved printer (Printers → Advanced) without waiting for a new built-in profile. **Status: implemented; proposed values are unverified.**

- **Scope.** Any saved printer using a built-in profile, including the verified SC03h. The built-in profile is never modified. An edit becomes that printer's own *custom profile* (`custom:<printer id>`, revision 1, 2, …), stored in the local database with the built-in profile it came from.
- **Editable:** `display_name`, `transport` (endpoints, write mode, chunk ceiling, pacing, timeouts), `raster`, `media`, `limits`, `driver` (heat per darkness level, speed, quality, feed and timing) and `default_density`.
- **Locked:** `id`, `protocol`, `match`, `vendor`, `model`, `verification`, `evidence`. A custom profile is matched only by the saved printer's own device ID and the built-in name rules, so it never takes part in discovery and never widens what NuPrint recognises.
- **Validation (Rust).** The edited profile passes the same checks as a built-in one (driver parameters, consistent dimensions, default darkness), plus limits that only apply to edits. Each rules out a value that stalls a job, prints nothing, or can only waste paper or overheat the head; staying inside them is not evidence a value is good. A test checks that every built-in profile passes them.

  | Area | Limit |
  | --- | --- |
  | Heat (each darkness level) | A quarter to twice the built-in **normal** heat (SC03h: 1250–10000) |
  | Speeds, feed scale | `print_speed`, `feed_speed`, `feed_steps_per_mm` at least 1 |
  | Extra feed | `feed.max_mm` at most 50 |
  | Status and readiness timing | `status_timeout_ms` at least 500, `ready_poll_ms` at least 100, `ready_deadline_ms` between the poll interval and 10 minutes, `settle_ms` at most 10 minutes |
  | Writes | `max_chunk_bytes` 20–512 (the negotiated write size still lowers it at run time); `chunk_delay_ms` and `boundary_delay_ms` at most 120 s |
  | Connection | every timeout 500 ms–120 s; `min_notify_subscriptions` at least 1, because without status notifications nothing can print |
  | Raster and media | `dots_per_mm` 1–50, `max_height` at most 20000, `wire_packing` unchanged, width a multiple of 8 and equal in `raster` and `driver` |
  | Other | `max_copies` at most 100; `display_name` 1–60 characters without control characters |

  The GATT service and notification checks still run before anything is sent.
- **Support level.** Always **Experimental** on every OS, with no verification entries, however the base profile is rated. Verification evidence is never inherited.
- **Revisions and jobs.** Each save is a new revision; every revision is kept. Queued jobs name the exact revision they were rendered for, so editing never changes a job already in the queue. A preview rendered for an older revision is rejected as stale and re-rendered.
- **Reset and export.** Reset points the printer back at the built-in profile (edited revisions stay stored for queued jobs). Saving settings identical to the built-in ones is a reset. Export writes the full profile as JSON with the portable ID `<base id>-custom` and no local printer ID; it is for sharing evidence toward a built-in profile. There is no import yet.
- **Evidence.** Treat edited values as *proposed*. A paper test of an edited profile moves nothing to verified; see the add-a-printer workflow above.

Adding another model under the same packet family tests profile reuse. Adding a model with a different protocol tests the adapter boundary. The second-model milestone should prefer the latter when suitable hardware is available. A fake second profile in tests is useful for architecture checks but cannot substitute for the physical acceptance criterion.

## Required regression cases

- Lowercase SC03h versus uppercase SC03H; equal names with different GATT layouts; shared UUIDs with ambiguous protocol evidence.
- AF30 advertisement followed by AE30 connection, and printers whose names are absent until connection.
- A second width, alternate bit polarity/order, models without status, and per-model write/MTU limits.
- Stale profile revisions, unexpected firmware, optional notifications missing, permissions denied and Bluetooth lost mid-job.
- No automatic protocol writes to unknown devices; profile selection changes preview and settings consistently.
- Existing SC03h vectors and the verified short-strip result remain unchanged during new-driver additions.
- Zero/reduced/default trailing feed preserves image bytes; profile switch updates valid range and total length; final feed is applied once across pages/copies and never injected after cancellation/failure. Unknown feed behavior is unavailable rather than borrowing SC03h values.
