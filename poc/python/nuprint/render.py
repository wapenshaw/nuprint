"""Preview and printing share the exact same monochrome bitmap."""

import os
from pathlib import Path
import re
from PIL import Image, ImageDraw, ImageFont, ImageOps
from .protocol import WIDTH, MAX_HEIGHT


def font(size: int):
    if not 12 <= size <= 64:
        raise ValueError("Font size must be 12–64 pixels")
    path = Path(os.environ.get("WINDIR", "C:/Windows")) / "Fonts" / "segoeui.ttf"
    if path.exists():
        return ImageFont.truetype(str(path), size)
    return ImageFont.load_default(size=size)


def prepare(image: Image.Image, dither: bool = True) -> Image.Image:
    image = ImageOps.exif_transpose(image).convert("RGBA")
    white = Image.new("RGBA", image.size, "white")
    white.alpha_composite(image)
    height = max(1, round(image.height * WIDTH / image.width))
    if height > MAX_HEIGHT:
        raise ValueError(f"This image is too long. Crop it to at most {MAX_HEIGHT} rows (about 50 cm).")
    gray = white.convert("L").resize((WIDTH, height), Image.Resampling.LANCZOS)
    if dither:
        return gray.convert("1", dither=Image.Dither.FLOYDSTEINBERG)
    return gray.point(lambda value: 255 if value >= 160 else 0, mode="1")


def text_image(text: str, size: int = 24) -> Image.Image:
    if not text.strip():
        raise ValueError("Type some text first")
    if len(text) > 20000:
        raise ValueError("Text is too long; split it into smaller prints")
    typeface = font(size)
    width = WIDTH - 24
    lines = []
    for paragraph in text.expandtabs(4).splitlines():
        current = ""
        for token in re.findall(r"\S+\s*|\s+", paragraph):
            if typeface.getlength(current + token.rstrip()) <= width:
                current += token
                continue
            if current.strip():
                lines.append(current.rstrip())
                current = ""
            for char in token:
                if typeface.getlength(current + char) > width:
                    lines.append(current.rstrip())
                    current = ""
                current += char
        lines.append(current.rstrip())
    ascent, descent = typeface.getmetrics()
    line_height = ascent + descent + 5
    height = 24 + line_height * len(lines)
    if height > MAX_HEIGHT:
        raise ValueError("Text is too long; split it into smaller prints")
    result = Image.new("L", (WIDTH, height), "white")
    draw = ImageDraw.Draw(result)
    for i, line in enumerate(lines):
        draw.text((12, 12 + i * line_height), line, font=typeface, fill=0)
    return prepare(result, dither=False)


def load_file(path: str, page: int = 1, dither: bool = True) -> tuple[Image.Image, int]:
    path = Path(path)
    if path.suffix.lower() == ".pdf":
        import pypdfium2 as pdfium
        with pdfium.PdfDocument(path) as doc:
            doc.init_forms()
            count = len(doc)
            if not 1 <= page <= count:
                raise ValueError(f"Choose a PDF page between 1 and {count}")
            pdfpage = doc[page - 1]
            try:
                if pdfpage.get_height() * WIDTH / pdfpage.get_width() > MAX_HEIGHT:
                    raise ValueError("PDF page is too long for one print")
                bitmap = pdfpage.render(scale=(WIDTH * 2) / pdfpage.get_width())
                try:
                    result = prepare(bitmap.to_pil(), dither)
                finally:
                    bitmap.close()
            finally:
                pdfpage.close()
        return result, count
    with Image.open(path) as image:
        return prepare(image, dither), 1


def test_image() -> Image.Image:
    image = text_image("ZEITEL / WINDOWS\nSC03h Bluetooth test\n0123456789\nLEFT                  RIGHT", size=22).convert("L")
    draw = ImageDraw.Draw(image)
    draw.rectangle((0, 0, WIDTH - 1, image.height - 1), outline=0)
    for x in range(12, WIDTH - 12, 8):
        draw.line((x, image.height - 9, x + 3, image.height - 9), fill=0, width=3)
    return image.convert("1")
