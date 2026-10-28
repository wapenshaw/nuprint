import argparse
import asyncio
import json
from pathlib import Path
from .bluetooth import DEFAULT_NAME, diagnose, print_image, scan
from .render import load_file, test_image, text_image
from .protocol import DEFAULT_FEED_MM, MAX_FEED_MM


def main(argv=None):
    parser = argparse.ArgumentParser(description="Print directly to a ZEITEL SC03h over Bluetooth LE")
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("scan", help="Find nearby SC03h printers")
    diag = commands.add_parser("diagnose", help="Connect and query status without printing")
    diag.add_argument("--device", default=DEFAULT_NAME)
    diag.add_argument("--output", type=Path, help="Save a diagnostic JSON report")
    for name in ("test", "text", "file"):
        sub = commands.add_parser(name)
        sub.add_argument("--device", default=DEFAULT_NAME)
        sub.add_argument("--darkness", choices=["light", "normal", "dark"], default="normal")
        sub.add_argument("--feed-mm", type=int, choices=range(MAX_FEED_MM + 1), default=DEFAULT_FEED_MM,
                         help="Extra paper after the image in nominal mm (default 12; 0 disables feed)")
        sub.add_argument("--preview", type=Path, help="Save PNG only; do not connect or print")
        if name == "text":
            sub.add_argument("text")
            sub.add_argument("--size", type=int, default=24)
        if name == "file":
            sub.add_argument("path")
            sub.add_argument("--page", type=int, default=1, help="PDF page, starting at 1")
            sub.add_argument("--threshold", action="store_true", help="Use crisp black/white instead of photo dithering")
    commands.add_parser("gui", help="Open the desktop application")
    args = parser.parse_args(argv)
    try:
        if args.command == "gui":
            from .gui import main as gui_main
            gui_main()
        elif args.command == "scan":
            devices = asyncio.run(scan())
            for device, adv in devices:
                print(f"{device.name}  {device.address}  signal {adv.rssi} dBm")
            if not devices:
                print("No SC03h found. Turn it on, close the phone app and check Windows Bluetooth.")
                return 1
        elif args.command == "diagnose":
            report = asyncio.run(diagnose(args.device))
            if args.output:
                args.output.parent.mkdir(parents=True, exist_ok=True)
                args.output.write_text(json.dumps(report, indent=2), encoding="utf-8")
        else:
            if args.command == "test":
                image = test_image()
            elif args.command == "text":
                image = text_image(args.text, args.size)
            else:
                image, _ = load_file(args.path, args.page, not args.threshold)
            if args.preview:
                args.preview.parent.mkdir(parents=True, exist_ok=True)
                image.save(args.preview, format="PNG")
                print(f"Preview saved: {args.preview} ({image.width} × {image.height})")
            else:
                asyncio.run(print_image(image, args.device, args.darkness, feed_mm=args.feed_mm))
        return 0
    except KeyboardInterrupt:
        print("Cancelled. Some data may already have reached the printer.")
        return 130
    except Exception as exc:
        print(f"Error: {exc or type(exc).__name__}")
        return 1
