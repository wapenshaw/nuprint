# Security policy

## Reporting a vulnerability

Please report security problems privately through GitHub's [private vulnerability reporting](https://github.com/wapenshaw/nuprint/security/advisories/new). Do not open a public issue for them. Include what you found, how to reproduce it and which version or commit you tested. You should get a first reply within a week.

Things worth reporting include:

- A way for the desktop frontend to reach files, Bluetooth or commands beyond the app's capabilities.
- A crafted image or PDF that crashes or takes over the renderer.
- Personal data (document content, PDF passwords, printer names or addresses) that is stored or exported when it shouldn't be, or a diagnostics export that is not redacted when it should be.

## Supported versions

Only the latest release and the `master` branch receive fixes.

## Scope

NuPrint talks to printers over unencrypted Bluetooth LE, like the vendor's own app. Someone nearby can watch or interfere with that traffic. This is a property of the printers and is not treated as a NuPrint vulnerability.
