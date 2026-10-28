//! Canonical monochrome raster shared by the renderer, preview and protocol drivers.
//!
//! Convention (docs/printer-support.md): row-major, **MSB-first** within each storage
//! byte, **1 = black**, row stride `ceil(width / 8)`, padding bits after the last pixel
//! are zero. Drivers transform this into their own wire order; for example SC03h rows
//! are LSB-first on the wire.

use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BitmapError {
    #[error("bitmap dimensions must be non-zero (got {width} x {height})")]
    Empty { width: u32, height: u32 },
    #[error("bitmap is too large ({width} x {height})")]
    TooLarge { width: u32, height: u32 },
    #[error("packed data has {actual} bytes; {width} x {height} needs {expected}")]
    Length { width: u32, height: u32, expected: usize, actual: usize },
    #[error("padding bits beyond the printable width must be zero")]
    DirtyPadding,
}

/// Upper bound on a single canonical page so a hostile input cannot request a huge allocation.
pub const MAX_PIXELS: u64 = 64 * 1024 * 1024;

#[derive(Clone, PartialEq, Eq)]
pub struct MonoBitmap {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

impl std::fmt::Debug for MonoBitmap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MonoBitmap")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("bytes", &self.data.len())
            .finish()
    }
}

impl MonoBitmap {
    pub fn stride_for(width: u32) -> usize {
        (width as usize).div_ceil(8)
    }

    fn check_dims(width: u32, height: u32) -> Result<(), BitmapError> {
        if width == 0 || height == 0 {
            return Err(BitmapError::Empty { width, height });
        }
        if u64::from(width) * u64::from(height) > MAX_PIXELS {
            return Err(BitmapError::TooLarge { width, height });
        }
        Ok(())
    }

    /// An all-white bitmap.
    pub fn new(width: u32, height: u32) -> Result<Self, BitmapError> {
        Self::check_dims(width, height)?;
        Ok(Self { width, height, data: vec![0; Self::stride_for(width) * height as usize] })
    }

    /// Wrap already-packed canonical data, validating length and zero padding.
    pub fn from_packed(width: u32, height: u32, data: Vec<u8>) -> Result<Self, BitmapError> {
        Self::check_dims(width, height)?;
        let stride = Self::stride_for(width);
        let expected = stride * height as usize;
        if data.len() != expected {
            return Err(BitmapError::Length { width, height, expected, actual: data.len() });
        }
        let used = width % 8;
        if used != 0 {
            let pad_mask = 0xFFu8 >> used;
            if data.chunks_exact(stride).any(|row| row[stride - 1] & pad_mask != 0) {
                return Err(BitmapError::DirtyPadding);
            }
        }
        Ok(Self { width, height, data })
    }

    /// Build from a per-pixel predicate (`true` = black). Intended for tests and synthetic patterns.
    pub fn from_fn(width: u32, height: u32, mut black: impl FnMut(u32, u32) -> bool) -> Result<Self, BitmapError> {
        let mut bitmap = Self::new(width, height)?;
        for y in 0..height {
            for x in 0..width {
                if black(x, y) {
                    bitmap.set(x, y, true);
                }
            }
        }
        Ok(bitmap)
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn stride(&self) -> usize {
        Self::stride_for(self.width)
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn rows(&self) -> impl ExactSizeIterator<Item = &[u8]> {
        self.data.chunks_exact(self.stride())
    }

    pub fn get(&self, x: u32, y: u32) -> bool {
        assert!(x < self.width && y < self.height, "pixel ({x}, {y}) outside bitmap");
        self.data[y as usize * self.stride() + (x / 8) as usize] & (0x80 >> (x % 8)) != 0
    }

    pub fn set(&mut self, x: u32, y: u32, black: bool) {
        assert!(x < self.width && y < self.height, "pixel ({x}, {y}) outside bitmap");
        let index = y as usize * self.stride() + (x / 8) as usize;
        let mask = 0x80 >> (x % 8);
        if black {
            self.data[index] |= mask;
        } else {
            self.data[index] &= !mask;
        }
    }

    /// Number of black pixels, useful for blank-output warnings.
    pub fn black_pixels(&self) -> u64 {
        self.data.iter().map(|b| u64::from(b.count_ones())).sum()
    }

    /// SHA-256 over width, height and packed rows: the identity of a canonical page.
    pub fn sha256_hex(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.width.to_le_bytes());
        hasher.update(self.height.to_le_bytes());
        hasher.update(&self.data);
        hex::encode(hasher.finalize())
    }

    /// SHA-256 of the packed rows only (matches the fixture generator's `rows_sha256`).
    pub fn rows_sha256_hex(&self) -> String {
        hex::encode(Sha256::digest(&self.data))
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;

    #[test]
    fn msb_first_one_is_black() {
        let mut b = MonoBitmap::new(10, 2).unwrap();
        b.set(0, 0, true);
        b.set(9, 1, true);
        assert_eq!(b.stride(), 2);
        assert_eq!(b.data(), &[0x80, 0x00, 0x00, 0x40]);
        assert!(b.get(0, 0) && b.get(9, 1) && !b.get(1, 0));
        assert_eq!(b.black_pixels(), 2);
    }

    #[test]
    fn rejects_bad_packed_data() {
        assert_matches!(MonoBitmap::from_packed(10, 1, vec![0]), Err(BitmapError::Length { .. }));
        assert_eq!(MonoBitmap::from_packed(10, 1, vec![0, 0x20]), Err(BitmapError::DirtyPadding));
        assert!(MonoBitmap::from_packed(10, 1, vec![0xFF, 0xC0]).is_ok());
        assert_matches!(MonoBitmap::new(0, 1), Err(BitmapError::Empty { .. }));
        assert_matches!(MonoBitmap::new(1 << 16, 1 << 16), Err(BitmapError::TooLarge { .. }));
    }
}
