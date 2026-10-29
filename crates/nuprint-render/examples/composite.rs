//! Time and peak heap of `render_image` for large pictures in each common pixel format.
//! cargo run -p nuprint-render --example composite --release

use std::alloc::{GlobalAlloc, Layout, System};
use std::io::Cursor;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use image::{DynamicImage, GrayImage, ImageFormat, Luma, Rgb, RgbImage, Rgba, RgbaImage};
use nuprint_render::{ImageParams, RenderTarget, render_image};

struct Peak;
static NOW: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Peak {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let n = NOW.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
        PEAK.fetch_max(n, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        NOW.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let n = NOW.fetch_add(new_size, Ordering::Relaxed) + new_size;
        PEAK.fetch_max(n, Ordering::Relaxed);
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        NOW.fetch_sub(layout.size(), Ordering::Relaxed);
        p
    }
}

#[global_allocator]
static A: Peak = Peak;

fn encode(image: &DynamicImage, format: ImageFormat) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    image.write_to(&mut out, format).unwrap();
    out.into_inner()
}

fn main() {
    let (w, h) = (6000u32, 6000u32);
    let v = |x: u32, y: u32| ((x * 255 / w) ^ (y * 255 / h)) as u8;
    let inputs = [
        (
            "RGBA PNG",
            encode(
                &DynamicImage::ImageRgba8(RgbaImage::from_fn(w, h, |x, y| Rgba([v(x, y), 90, 200, v(y, x)]))),
                ImageFormat::Png,
            ),
        ),
        (
            "RGB JPEG",
            encode(
                &DynamicImage::ImageRgb8(RgbImage::from_fn(w, h, |x, y| Rgb([v(x, y), 90, 200]))),
                ImageFormat::Jpeg,
            ),
        ),
        (
            "grey PNG",
            encode(&DynamicImage::ImageLuma8(GrayImage::from_fn(w, h, |x, y| Luma([v(x, y)]))), ImageFormat::Png),
        ),
    ];
    let params = ImageParams::default();
    let target = RenderTarget::default();
    println!("{:<10} {:>8} {:>10} {:>10}", "input", "MiB in", "median ms", "peak MiB");
    for (name, bytes) in &inputs {
        let mut times = Vec::new();
        let mut peak = 0;
        for _ in 0..7 {
            let base = NOW.load(Ordering::Relaxed);
            PEAK.store(base, Ordering::Relaxed);
            let start = Instant::now();
            let out = render_image(bytes, &params, &target).unwrap();
            times.push(start.elapsed().as_secs_f64() * 1000.0);
            drop(out);
            peak = peak.max(PEAK.load(Ordering::Relaxed) - base);
        }
        times.sort_by(f64::total_cmp);
        let mib = |n: usize| n as f64 / 1048576.0;
        println!("{name:<10} {:>8.1} {:>10.0} {:>10.0}", mib(bytes.len()), times[3], mib(peak));
    }
}
