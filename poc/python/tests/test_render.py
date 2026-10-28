import pytest
from PIL import Image
from nuprint.render import load_file, prepare, test_image as make_test_image, text_image


def test_transparency_prints_as_white():
    result = prepare(Image.new("RGBA", (384, 2), (0, 0, 0, 0)))
    assert result.mode == "1"
    assert result.getextrema() == (255, 255)


def test_scaling_preserves_aspect_ratio():
    assert prepare(Image.new("RGB", (768, 100))).size == (384, 50)


def test_text_wraps_long_word_and_contains_ink():
    result = text_image("W" * 80, 24)
    assert result.width == 384 and result.height > 100
    assert result.getextrema() == (0, 255)


@pytest.mark.parametrize("text,size", [("", 24), ("ok", 1), ("a\n" * 1000, 24)])
def test_invalid_text(text, size):
    with pytest.raises(ValueError):
        text_image(text, size)


def test_image_roundtrip(tmp_path):
    path = tmp_path / "test.png"
    make_test_image().save(path)
    image, pages = load_file(str(path), dither=False)
    assert image.tobytes() == make_test_image().tobytes()
    assert pages == 1


def test_pdf_page_selection_and_bounds(tmp_path):
    path = tmp_path / "two-pages.pdf"
    red = Image.new("RGB", (384, 100), "black")
    white = Image.new("RGB", (384, 200), "white")
    red.save(path, format="PDF", save_all=True, append_images=[white])
    first, count = load_file(str(path), page=1)
    second, _ = load_file(str(path), page=2)
    assert count == 2
    assert first.size == (384, 100) and first.getextrema() == (0, 0)
    assert second.size == (384, 200) and second.getextrema() == (255, 255)
    with pytest.raises(ValueError, match="between 1 and 2"):
        load_file(str(path), page=3)
