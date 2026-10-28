"""Native desktop UI. All Tk calls stay on the main thread; BLE runs off-thread."""

import asyncio
from datetime import datetime
import os
from pathlib import Path
import queue
import threading
import tkinter as tk
from tkinter import filedialog, messagebox, ttk
from PIL import ImageTk
from . import __version__
from .bluetooth import DEFAULT_NAME, diagnose, print_image, scan
from .render import load_file, test_image, text_image
from .protocol import DEFAULT_FEED_MM, MAX_FEED_MM, feed_steps


class App:
    def __init__(self, root):
        self.root = root
        root.title("NuPrint · SC03h")
        root.geometry("1050x830")
        root.minsize(900, 790)
        root.configure(bg="#f4f3ef")
        self.events = queue.Queue()
        self.cancel_requested = threading.Event()
        self.busy = False
        self.closing = False
        self.file_path = None
        self.image = None
        self.source = "text"
        self.device = tk.StringVar(value=DEFAULT_NAME)
        self.darkness = tk.StringVar(value="normal")
        self.feed_mm = tk.StringVar(value=str(DEFAULT_FEED_MM))
        self.size = tk.StringVar(value="24")
        self.page = tk.StringVar(value="1")
        self.dither = tk.BooleanVar(value=True)
        self.status = tk.StringVar(value="Ready to connect · turn on your printer")
        self.detail = tk.StringVar()
        self.controls = []
        self.log_path = Path(os.environ.get("LOCALAPPDATA", str(Path.home()))) / "NuPrint" / "nuprint.log"
        self.log_path.parent.mkdir(parents=True, exist_ok=True)
        if self.log_path.exists() and self.log_path.stat().st_size > 2_000_000:
            self.log_path.write_text("", encoding="utf-8")
        self.build()
        self.feed_mm.trace_add("write", self.update_detail)
        self.preview()
        self.root.protocol("WM_DELETE_WINDOW", self.close)
        self.root.after(80, self.poll)

    def button(self, parent, text, command, **kwargs):
        button = ttk.Button(parent, text=text, command=command, **kwargs)
        self.controls.append(button)
        return button

    def build(self):
        style = ttk.Style()
        style.theme_use("clam")
        style.configure("TFrame", background="#f4f3ef")
        style.configure("TLabel", background="#f4f3ef", foreground="#202923", font=("Segoe UI", 10))
        style.configure("Title.TLabel", font=("Segoe UI Semibold", 25))
        style.configure("Small.TLabel", foreground="#68736b", font=("Segoe UI", 9))
        style.configure("TButton", font=("Segoe UI", 10), padding=(12, 8))
        style.configure("Print.TButton", background="#21563e", foreground="white", font=("Segoe UI Semibold", 12))
        style.map("Print.TButton", background=[("active", "#2d7253"), ("disabled", "#a3b0a7")])
        outer = ttk.Frame(self.root, padding=24)
        outer.pack(fill="both", expand=True)
        ttk.Label(outer, text="NuPrint", style="Title.TLabel").pack(anchor="w")
        ttk.Label(outer, text="Small printer. Straight from your desktop.", style="Small.TLabel").pack(anchor="w", pady=(0, 18))
        connection = ttk.Frame(outer)
        connection.pack(fill="x", pady=(0, 18))
        ttk.Label(connection, text="Printer").pack(side="left", padx=(0, 10))
        self.devices = ttk.Combobox(connection, textvariable=self.device, width=27, values=[DEFAULT_NAME])
        self.devices.pack(side="left", padx=(0, 8))
        self.controls.append(self.devices)
        self.button(connection, "Find printer", self.find).pack(side="left", padx=(0, 8))
        self.button(connection, "Check connection", self.check_connection).pack(side="left")
        ttk.Label(connection, text="Bluetooth LE · SC03h", style="Small.TLabel").pack(side="right")

        body = ttk.Frame(outer)
        body.pack(fill="both", expand=True)
        left = ttk.Frame(body, padding=(0, 0, 22, 0))
        left.pack(side="left", fill="both", expand=True)
        left.columnconfigure(0, weight=1)
        left.rowconfigure(1, weight=1)
        ttk.Label(left, text="What would you like to print?", font=("Segoe UI Semibold", 13)).grid(row=0, column=0, sticky="w", pady=(0, 10))
        self.tabs = ttk.Notebook(left)
        self.tabs.grid(row=1, column=0, sticky="nsew")
        text_tab = ttk.Frame(self.tabs, padding=12)
        text_tab.columnconfigure(0, weight=1)
        text_tab.rowconfigure(0, weight=1)
        file_tab = ttk.Frame(self.tabs, padding=12)
        self.tabs.add(text_tab, text="  Text  ")
        self.tabs.add(file_tab, text="  Image / PDF  ")
        self.text = tk.Text(text_tab, wrap="word", width=32, height=5, font=("Segoe UI", 12),
                            relief="flat", padx=12, pady=12, undo=True, background="white")
        self.text.grid(row=0, column=0, sticky="nsew")
        self.text.insert("1.0", "Hello from Windows!\n\nYour SC03h is ready to print.")
        self.text.bind("<KeyRelease>", self.source_changed)
        options = ttk.Frame(text_tab)
        options.grid(row=1, column=0, sticky="ew", pady=(12, 0))
        ttk.Label(options, text="Text size").pack(side="left")
        sizes = ttk.Combobox(options, textvariable=self.size, values=[18, 22, 24, 28, 32, 40, 48], width=5, state="readonly")
        sizes.pack(side="left", padx=8)
        self.controls.append(sizes)
        self.button(file_tab, "Open image or PDF…", self.open_file).pack(anchor="w")
        self.file_label = ttk.Label(file_tab, text="PNG, JPEG, BMP, WebP, TIFF or PDF", wraplength=330)
        self.file_label.pack(anchor="w", pady=14)
        page_row = ttk.Frame(file_tab)
        page_row.pack(fill="x", pady=8)
        ttk.Label(page_row, text="PDF page").pack(side="left")
        self.page_input = ttk.Spinbox(page_row, from_=1, to=1, textvariable=self.page, width=5)
        self.page_input.pack(side="left", padx=8)
        self.controls.append(self.page_input)
        self.page_count = ttk.Label(page_row, text="")
        self.page_count.pack(side="left")
        dithering = ttk.Checkbutton(file_tab, text="Photo shading (dithering)", variable=self.dither)
        dithering.pack(anchor="w", pady=8)
        self.controls.append(dithering)
        ttk.Label(file_tab, text="One PDF page per print. Full-size documents shrink to the narrow paper; use large text for readable results.",
                  wraplength=325, style="Small.TLabel").pack(anchor="w", pady=12)

        actions = ttk.Frame(left)
        actions.grid(row=2, column=0, sticky="ew", pady=(10, 6))
        self.button(actions, "Update preview", self.preview).pack(side="left")
        self.button(actions, "Test pattern", self.show_test).pack(side="left", padx=8)
        density = ttk.Frame(left)
        density.grid(row=3, column=0, sticky="ew", pady=(4, 8))
        ttk.Label(density, text="Darkness").pack(side="left")
        darkness = ttk.Combobox(density, textvariable=self.darkness, values=["light", "normal", "dark"], width=9, state="readonly")
        darkness.pack(side="left", padx=8)
        self.controls.append(darkness)
        feed_row = ttk.Frame(left)
        feed_row.grid(row=4, column=0, sticky="ew", pady=(0, 8))
        ttk.Label(feed_row, text="Extra feed (mm)").pack(side="left")
        self.feed_input = ttk.Spinbox(feed_row, from_=0, to=MAX_FEED_MM, textvariable=self.feed_mm, width=4)
        self.feed_input.pack(side="left", padx=8)
        self.controls.append(self.feed_input)
        ttk.Label(feed_row, text="0 = none · 12 = original", style="Small.TLabel").pack(side="left")
        self.button(left, "Print", self.print_current, style="Print.TButton").grid(row=5, column=0, sticky="ew")
        self.cancel_button = ttk.Button(left, text="Cancel transfer", command=self.cancel, state="disabled")
        self.cancel_button.grid(row=6, column=0, sticky="ew", pady=(6, 0))
        ttk.Label(left, text="Less feed may leave the last line inside the printer.\nKeep the phone's printer app closed.", style="Small.TLabel").grid(row=7, column=0, sticky="w", pady=(8, 0))

        right = ttk.Frame(body, width=420)
        right.pack(side="right", fill="both")
        right.pack_propagate(False)
        ttk.Label(right, text="PAPER PREVIEW", style="Small.TLabel").pack(anchor="w", pady=(0, 10))
        preview_frame = ttk.Frame(right)
        preview_frame.pack(fill="both", expand=True)
        self.canvas = tk.Canvas(preview_frame, width=400, bg="#dedfd8", highlightthickness=0)
        scroll = ttk.Scrollbar(preview_frame, orient="vertical", command=self.canvas.yview)
        self.canvas.configure(yscrollcommand=scroll.set)
        scroll.pack(side="right", fill="y")
        self.canvas.pack(fill="both", expand=True)
        self.canvas.bind("<MouseWheel>", lambda e: self.canvas.yview_scroll(-int(e.delta / 120), "units"))
        ttk.Label(right, textvariable=self.detail, style="Small.TLabel").pack(anchor="w", pady=8)
        self.button(right, "Save preview…", self.save_preview).pack(anchor="w")

        footer = ttk.Frame(outer)
        footer.pack(side="bottom", fill="x")
        body.pack_forget()
        body.pack(fill="both", expand=True)
        self.progress = ttk.Progressbar(footer, maximum=100)
        self.progress.pack(fill="x", pady=(16, 8))
        ttk.Label(footer, textvariable=self.status, wraplength=970).pack(anchor="w")
        self.log_box = tk.Text(footer, height=3, font=("Consolas", 9), state="disabled", relief="flat", bg="#e8e9e3", fg="#455249")
        self.log_box.pack(fill="x", pady=(8, 0))
        ttk.Label(footer, text=f"v{__version__}  ·  384 dots  ·  Continuous thermal paper", style="Small.TLabel").pack(anchor="w", pady=(6, 0))
        self.tabs.bind("<<NotebookTabChanged>>", self.source_changed)

    def source_changed(self, _event=None):
        self.source = "text" if self.tabs.index("current") == 0 else "file"

    def render_current(self):
        if self.source == "test":
            return test_image()
        if self.tabs.index("current") == 0:
            return text_image(self.text.get("1.0", "end-1c"), int(self.size.get()))
        if not self.file_path:
            raise ValueError("Open an image or PDF first")
        image, count = load_file(self.file_path, int(self.page.get()), self.dither.get())
        self.page_input.configure(to=count)
        self.page_count.configure(text=f"of {count}")
        return image

    def display(self, image):
        self.image = image
        self.photo = ImageTk.PhotoImage(image)
        self.canvas.delete("all")
        self.canvas.create_image(8, 12, anchor="nw", image=self.photo)
        self.canvas.configure(scrollregion=(0, 0, 400, image.height + 24))
        self.canvas.yview_moveto(0)
        self.update_detail()

    def selected_feed_mm(self):
        try:
            value = int(self.feed_mm.get())
        except ValueError:
            raise ValueError(f"Extra feed must be a whole number from 0 to {MAX_FEED_MM} mm") from None
        feed_steps(value)
        return value

    def update_detail(self, *_):
        if self.image is None:
            return
        try:
            distance = self.selected_feed_mm()
            self.detail.set(f"384 × {self.image.height} dots · about {self.image.height / 8 + distance:.1f} mm total\nExtra feed: {distance} mm (not included in PNG)")
        except ValueError as exc:
            self.detail.set(str(exc))

    def preview(self):
        try:
            self.source_changed()
            self.display(self.render_current())
        except Exception as exc:
            messagebox.showerror("Preview", str(exc), parent=self.root)

    def show_test(self):
        self.source = "test"
        self.display(test_image())
        self.status.set("Test pattern preview · press Print to send it")

    def open_file(self):
        path = filedialog.askopenfilename(filetypes=[("Images and PDFs", "*.png *.jpg *.jpeg *.bmp *.webp *.tif *.tiff *.pdf"), ("All files", "*.*")])
        if path:
            self.file_path = path
            self.file_label.configure(text=Path(path).name)
            self.page.set("1")
            self.preview()

    def save_preview(self):
        if self.source != "test":
            try:
                self.display(self.render_current())
            except Exception as exc:
                messagebox.showerror("Preview", str(exc), parent=self.root)
                return
        path = filedialog.asksaveasfilename(defaultextension=".png", initialfile="nuprint-preview.png", filetypes=[("PNG", "*.png")])
        if path:
            try:
                self.image.save(path)
            except Exception as exc:
                messagebox.showerror("Save preview", str(exc), parent=self.root)

    def log(self, message):
        self.events.put(("log", message))

    def begin(self, factory, message):
        if self.busy:
            return
        self.busy = True
        self.cancel_requested.clear()
        for widget in self.controls:
            widget.state(["disabled"])
        self.cancel_button.state(["!disabled"])
        self.progress["value"] = 0
        self.status.set(message)

        async def operation():
            task = asyncio.create_task(factory())
            try:
                while not task.done():
                    if self.cancel_requested.is_set():
                        task.cancel()
                        break
                    await asyncio.wait({task}, timeout=0.1)
                return await task
            finally:
                if not task.done():
                    task.cancel()
                    await asyncio.gather(task, return_exceptions=True)

        def worker():
            try:
                asyncio.run(operation())
            except asyncio.CancelledError:
                self.events.put(("status", "Cancelled. Data already in the printer may still print."))
            except Exception as exc:
                self.events.put(("error", str(exc) or type(exc).__name__))
            finally:
                self.events.put(("done", None))
        threading.Thread(target=worker, daemon=True).start()

    def find(self):
        async def work():
            devices = await scan()
            names = [device.name for device, _ in devices]
            self.events.put(("devices", names))
            self.events.put(("status", f"Found {len(names)} SC03h printer(s)" if names else "No SC03h found. Turn it on and check Windows Bluetooth."))
        self.begin(work, "Scanning for SC03h printers…")

    def check_connection(self):
        target = self.device.get().strip()
        async def work():
            result = await diagnose(target, self.log)
            self.events.put(("status", f"Connection verified · {result['status_text']}"))
        self.begin(work, "Checking Bluetooth connection…")

    def print_current(self):
        try:
            image = self.render_current()
            self.display(image)
            target = self.device.get().strip()
            darkness = self.darkness.get()
            feed_mm = self.selected_feed_mm()
        except Exception as exc:
            messagebox.showerror("Print", str(exc), parent=self.root)
            return
        async def work():
            await print_image(image, target, darkness, self.log,
                              lambda sent, total: self.events.put(("progress", sent * 100 / total)),
                              feed_mm=feed_mm)
            self.events.put(("status", "Transfer complete · printer reports ready"))
        self.begin(work, "Connecting to print…")

    def cancel(self):
        self.cancel_requested.set()
        self.cancel_button.state(["disabled"])
        self.status.set("Cancelling and disconnecting…")

    def poll(self):
        while True:
            try:
                event, value = self.events.get_nowait()
            except queue.Empty:
                break
            if event == "log":
                self.log_box.configure(state="normal")
                self.log_box.insert("end", value + "\n")
                self.log_box.see("end")
                self.log_box.configure(state="disabled")
                try:
                    with self.log_path.open("a", encoding="utf-8") as stream:
                        stream.write(f"{datetime.now().isoformat(timespec='seconds')} {value}\n")
                except OSError:
                    pass
            elif event == "devices":
                self.devices.configure(values=value)
                if value and self.device.get() not in value:
                    self.device.set(value[0])
            elif event == "status":
                self.status.set(value)
            elif event == "progress":
                self.progress["value"] = value
            elif event == "error":
                self.status.set(value)
                self.log("ERROR: " + value)
                if not self.closing:
                    messagebox.showerror("NuPrint", value, parent=self.root)
            elif event == "done":
                self.busy = False
                for widget in self.controls:
                    widget.state(["!disabled"])
                self.cancel_button.state(["disabled"])
        if self.closing and not self.busy:
            self.root.destroy()
            return
        self.root.after(80, self.poll)

    def close(self):
        self.closing = True
        if self.busy:
            self.cancel()
        else:
            self.root.destroy()


def main():
    root = tk.Tk()
    App(root)
    root.mainloop()


if __name__ == "__main__":
    main()
