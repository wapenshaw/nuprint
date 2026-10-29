//! `iprint-5178`: the `51 78`-framed packet family used by the lowercase SC03h.
//!
//! Independently implemented from APK observations and hardware replies recorded in
//! docs/protocol.md. Model values (width, heat, speeds, feed scale) come from the
//! profile via [`Iprint5178Params`]; nothing here is a global default for other models.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::MonoBitmap;
use crate::driver::{
    CompletionPolicy, Density, DeviceInfo, DiagnosticQuery, ExchangePlan, FeedCapability, NotificationDecoder,
    PlanStep, PrintRequest, PrinterDriver, ProtocolError, ProtocolEvent, StatusFlag, StatusReport,
};

pub const PROTOCOL_ID: &str = "iprint-5178";
pub const PROTOCOL_VERSION: u32 = 1;
pub const MAGIC: [u8; 2] = [0x51, 0x78];
/// Bytes a frame adds around its payload: magic, opcode, direction, length, CRC and terminator.
const FRAME_OVERHEAD: usize = 8;
/// Application guard for notification payloads; the wire length field is uint16.
pub const MAX_NOTIFICATION_PAYLOAD: usize = 512;

pub mod opcode {
    pub const FEED: u8 = 0xA1;
    pub const RASTER_ROW: u8 = 0xA2;
    pub const STATUS: u8 = 0xA3;
    pub const QUALITY: u8 = 0xA4;
    pub const DEVICE_INFO: u8 = 0xA8;
    pub const FLOW: u8 = 0xAE;
    pub const ENERGY: u8 = 0xAF;
    pub const SPEED: u8 = 0xBD;
    pub const MODE: u8 = 0xBE;
}

pub const DIRECTION_COMMAND: u8 = 0x00;
pub const DIRECTION_RESPONSE: u8 = 0x01;

/// CRC-8, polynomial 0x07, init 0, no reflection, no final XOR. `crc8(b"123456789") == 0xF4`.
pub fn crc8(data: &[u8]) -> u8 {
    data.iter().fold(0, |crc, &value| CRC8_TABLE[usize::from(crc ^ value)])
}

/// [`crc8`] of every single byte, so each payload byte costs one lookup.
const CRC8_TABLE: [u8; 256] = {
    let mut table = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u8;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 0x80 != 0 { (crc << 1) ^ 0x07 } else { crc << 1 };
            bit += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
};

/// `51 78 | opcode | direction | len u16 LE | payload | crc8(payload) | FF`
pub fn frame_with_direction(opcode: u8, direction: u8, payload: &[u8]) -> Result<Vec<u8>, ProtocolError> {
    let len = u16::try_from(payload.len()).map_err(|_| ProtocolError::PayloadTooLarge(payload.len()))?;
    let mut out = Vec::with_capacity(payload.len() + FRAME_OVERHEAD);
    out.extend_from_slice(&MAGIC);
    out.push(opcode);
    out.push(direction);
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(payload);
    out.push(crc8(payload));
    out.push(0xFF);
    Ok(out)
}

/// Command frame. Panics only for payloads over 65535 bytes, which this crate never builds.
pub fn command(opcode: u8, payload: &[u8]) -> Vec<u8> {
    frame_with_direction(opcode, DIRECTION_COMMAND, payload).expect("command payload fits uint16")
}

/// Append a command frame to `out`, with `map` applied to each payload byte on the way in.
/// Panics only for payloads over 65535 bytes, like [`command`].
fn push_command(out: &mut Vec<u8>, opcode: u8, payload: &[u8], map: impl Fn(u8) -> u8) {
    let len = u16::try_from(payload.len()).expect("command payload fits uint16");
    out.extend_from_slice(&MAGIC);
    out.push(opcode);
    out.push(DIRECTION_COMMAND);
    out.extend_from_slice(&len.to_le_bytes());
    let start = out.len();
    out.extend(payload.iter().map(|&b| map(b)));
    let crc = crc8(&out[start..]);
    out.push(crc);
    out.push(0xFF);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub opcode: u8,
    pub direction: u8,
    pub payload: Vec<u8>,
}

/// Reassembles fragmented/concatenated notifications; skips noise and CRC/terminator errors.
#[derive(Debug, Default)]
pub struct FrameDecoder {
    buffer: Vec<u8>,
}

impl FrameDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn feed(&mut self, data: &[u8]) -> Vec<Frame> {
        self.buffer.extend_from_slice(data);
        let mut frames = Vec::new();
        loop {
            let Some(start) = self.buffer.array_windows().position(|w| *w == MAGIC) else {
                // Keep a trailing 0x51 that may begin the next frame.
                let keep = self.buffer.last() == Some(&MAGIC[0]);
                self.buffer.clear();
                if keep {
                    self.buffer.push(MAGIC[0]);
                }
                break;
            };
            self.buffer.drain(..start);
            if self.buffer.len() < 6 {
                break;
            }
            let size = usize::from(u16::from_le_bytes([self.buffer[4], self.buffer[5]]));
            if size > MAX_NOTIFICATION_PAYLOAD {
                self.buffer.remove(0);
                continue;
            }
            if self.buffer.len() < size + 8 {
                break;
            }
            let payload = &self.buffer[6..6 + size];
            if self.buffer[7 + size] != 0xFF || crc8(payload) != self.buffer[6 + size] {
                self.buffer.remove(0);
                continue;
            }
            frames.push(Frame { opcode: self.buffer[2], direction: self.buffer[3], payload: payload.to_vec() });
            self.buffer.drain(..size + 8);
        }
        frames
    }
}

/// APK status bit meanings for this family.
pub fn status_flags(value: u8) -> Vec<StatusFlag> {
    [
        (0x01, StatusFlag::PaperOut),
        (0x02, StatusFlag::CoverOpen),
        (0x04, StatusFlag::Overheated),
        (0x08, StatusFlag::LowPower),
        (0x10, StatusFlag::Charging),
        (0x80, StatusFlag::Printing),
    ]
    .into_iter()
    .filter(|(bit, _)| value & bit != 0)
    .map(|(_, flag)| flag)
    .collect()
}

/// Longest printable-ASCII run of at least three bytes, e.g. `3.0.49` in the A8 reply.
fn parse_firmware(payload: &[u8]) -> Option<String> {
    payload
        .split(|b| !(0x20..=0x7E).contains(b))
        .filter(|run| run.len() >= 3 && run.iter().any(u8::is_ascii_digit))
        .max_by_key(|run| run.len())
        .map(|run| String::from_utf8_lossy(run).into_owned())
}

pub struct Iprint5178Decoder {
    frames: FrameDecoder,
}

impl NotificationDecoder for Iprint5178Decoder {
    fn feed(&mut self, data: &[u8]) -> Vec<ProtocolEvent> {
        self.frames
            .feed(data)
            .into_iter()
            .filter(|f| f.direction == DIRECTION_RESPONSE)
            .filter_map(|f| match (f.opcode, f.payload.first().copied()) {
                (opcode::FLOW, Some(0x10)) => Some(ProtocolEvent::FlowPause),
                (opcode::FLOW, Some(0x00)) => Some(ProtocolEvent::FlowResume),
                (opcode::STATUS, Some(value)) => {
                    Some(ProtocolEvent::Status(StatusReport { flags: status_flags(value), raw: f.payload }))
                }
                (opcode::DEVICE_INFO, Some(_)) => {
                    Some(ProtocolEvent::DeviceInfo(DeviceInfo { firmware: parse_firmware(&f.payload), raw: f.payload }))
                }
                (_, None) => None,
                (op, Some(_)) => Some(ProtocolEvent::Unhandled { opcode: op, payload: f.payload }),
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Iprint5178Timing {
    pub status_timeout_ms: u64,
    pub settle_ms: u64,
    pub ready_poll_ms: u64,
    pub ready_deadline_ms: u64,
}

/// Model parameters supplied by a reviewed profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Iprint5178Params {
    pub width_dots: u32,
    pub max_height: u32,
    /// A4 payload; the APK's quality-3 setting is 0x33.
    pub quality: u8,
    /// AF energy per named preset, little-endian on the wire.
    pub energy: BTreeMap<Density, u16>,
    /// BE payload; 0 = image mode.
    pub image_mode: u8,
    /// BD payload before raster rows.
    pub print_speed: u8,
    /// BD payload before trailing feed.
    pub feed_speed: u8,
    pub feed_steps_per_mm: u16,
    /// Largest A1 distance per command; the verified sequence used two 48-step commands.
    pub feed_command_max_steps: u16,
    pub feed: FeedCapability,
    pub timing: Iprint5178Timing,
}

impl Iprint5178Params {
    /// Values for the verified lowercase SC03h (docs/protocol.md). Profiles should supply
    /// these explicitly; this exists for tests and as a review reference.
    pub fn sc03h_reference() -> Self {
        Self {
            width_dots: 384,
            max_height: 4000,
            quality: 0x33,
            energy: BTreeMap::from([(Density::Light, 4250), (Density::Normal, 5000), (Density::Dark, 5750)]),
            image_mode: 0x00,
            print_speed: 10,
            feed_speed: 25,
            feed_steps_per_mm: 8,
            feed_command_max_steps: 48,
            feed: FeedCapability { min_mm: 0, max_mm: 12, default_mm: 12, calibrated_reduced: false },
            timing: Iprint5178Timing {
                status_timeout_ms: 3000,
                settle_ms: 2000,
                ready_poll_ms: 1000,
                ready_deadline_ms: 60_000,
            },
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.width_dots == 0 || !self.width_dots.is_multiple_of(8) {
            return Err("iprint-5178 width must be a non-zero multiple of 8 dots".into());
        }
        if self.max_height == 0 || self.energy.is_empty() || !self.energy.contains_key(&Density::Normal) {
            return Err("iprint-5178 profile needs a height bound and a normal energy preset".into());
        }
        if self.feed_command_max_steps == 0 || self.feed.min_mm > self.feed.max_mm {
            return Err("iprint-5178 feed capability is inconsistent".into());
        }
        if !self.feed.contains(self.feed.default_mm) {
            return Err("iprint-5178 default feed is outside its bounds".into());
        }
        let max_steps = u32::from(self.feed.max_mm) * u32::from(self.feed_steps_per_mm);
        if max_steps > u32::from(u16::MAX) {
            return Err("iprint-5178 feed bound exceeds the uint16 distance".into());
        }
        Ok(())
    }

    /// Extra limits for values an owner edited, on top of [`Self::validate`]. `base` holds the
    /// reviewed values the edit started from. Each limit rules out a value that stalls the job,
    /// prints nothing, or can only waste paper or overheat the head; none of them is a claim
    /// that every value inside the limits is good.
    pub fn check_edit(&self, base: &Self) -> Result<(), String> {
        let normal = u32::from(base.energy.get(&Density::Normal).copied().unwrap_or(0));
        let (lowest, highest) = (normal / HEAT_FLOOR_DIVISOR, normal * HEAT_CEILING_FACTOR);
        for (density, &heat) in &self.energy {
            if !(lowest..=highest).contains(&u32::from(heat)) {
                return Err(format!(
                    "heat for {} must be {lowest}–{highest}: a quarter to twice the built-in normal heat of {normal}.",
                    density.as_str()
                ));
            }
        }
        if self.print_speed == 0 || self.feed_speed == 0 {
            return Err("print_speed and feed_speed must be at least 1.".into());
        }
        if self.feed_steps_per_mm == 0 {
            return Err("feed_steps_per_mm must be at least 1.".into());
        }
        if self.feed.max_mm > MAX_EDITED_FEED_MM {
            return Err(format!("feed.max_mm must be at most {MAX_EDITED_FEED_MM}."));
        }
        let t = &self.timing;
        if !(MIN_EDITED_WAIT_MS..=MAX_EDITED_STATUS_WAIT_MS).contains(&t.status_timeout_ms)
            || t.ready_poll_ms < MIN_EDITED_POLL_MS
        {
            return Err(format!(
                "status_timeout_ms must be {MIN_EDITED_WAIT_MS}–{MAX_EDITED_STATUS_WAIT_MS} and ready_poll_ms at least {MIN_EDITED_POLL_MS}."
            ));
        }
        if t.ready_deadline_ms < t.ready_poll_ms || t.ready_deadline_ms > MAX_EDITED_READY_DEADLINE_MS {
            return Err(format!("ready_deadline_ms must be from ready_poll_ms up to {MAX_EDITED_READY_DEADLINE_MS}."));
        }
        if t.settle_ms > MAX_EDITED_READY_DEADLINE_MS {
            return Err(format!("settle_ms must be at most {MAX_EDITED_READY_DEADLINE_MS}."));
        }
        Ok(())
    }
}

/// Edited heat may not fall below a quarter of the built-in normal heat...
const HEAT_FLOOR_DIVISOR: u32 = 4;
/// ...or rise above twice it.
const HEAT_CEILING_FACTOR: u32 = 2;
/// More than this much extra paper per job is never intended.
const MAX_EDITED_FEED_MM: u16 = 50;
/// A status reply cannot arrive faster than this...
const MIN_EDITED_WAIT_MS: u64 = 500;
/// ...and one that has not arrived after this long is not coming.
const MAX_EDITED_STATUS_WAIT_MS: u64 = 60_000;
/// Polling faster than this only spins.
const MIN_EDITED_POLL_MS: u64 = 100;
const MAX_EDITED_READY_DEADLINE_MS: u64 = 600_000;

pub struct Iprint5178Driver {
    params: Iprint5178Params,
}

impl Iprint5178Driver {
    pub fn new(params: Iprint5178Params) -> Result<Self, String> {
        params.validate()?;
        Ok(Self { params })
    }

    pub fn feed_steps(&self, feed_mm: u16) -> Result<u16, ProtocolError> {
        let feed = self.params.feed;
        if !feed.contains(feed_mm) {
            return Err(ProtocolError::FeedOutOfRange { min: feed.min_mm, max: feed.max_mm });
        }
        Ok(feed_mm * self.params.feed_steps_per_mm)
    }

    /// Canonical MSB-first/1=black rows → wire LSB-first/1=black rows.
    pub fn wire_rows(&self, request: &PrintRequest<'_>) -> Result<Vec<Vec<u8>>, ProtocolError> {
        self.check_raster(request.bitmap)?;
        Ok(request.bitmap.rows().map(|row| row.iter().map(|b| b.reverse_bits()).collect()).collect())
    }

    fn check_raster(&self, bitmap: &MonoBitmap) -> Result<(), ProtocolError> {
        if bitmap.width() != self.params.width_dots || !(1..=self.params.max_height).contains(&bitmap.height()) {
            return Err(ProtocolError::InvalidRaster(format!(
                "Expected a monochrome {}-dot-wide image, 1–{} rows (got {} × {})",
                self.params.width_dots,
                self.params.max_height,
                bitmap.width(),
                bitmap.height()
            )));
        }
        Ok(())
    }

    /// Exact byte stream for one page, identical to the Python reference `print_job()`.
    pub fn encode_job(&self, request: &PrintRequest<'_>) -> Result<Vec<u8>, ProtocolError> {
        let steps = self.feed_steps(request.trailing_feed_mm)?;
        let energy = *self
            .params
            .energy
            .get(&request.density)
            .ok_or_else(|| ProtocolError::UnsupportedDensity(request.density.as_str().into()))?;
        let bitmap = request.bitmap;
        self.check_raster(bitmap)?; // Validate completely before producing anything.
        let feeds = usize::from(steps.div_ceil(self.params.feed_command_max_steps));
        let row_frame = bitmap.stride() + FRAME_OVERHEAD;
        let mut out = Vec::with_capacity(bitmap.height() as usize * row_frame + (6 + feeds) * (FRAME_OVERHEAD + 2));
        let same = |b: u8| b;
        push_command(&mut out, opcode::QUALITY, &[self.params.quality], same);
        push_command(&mut out, opcode::ENERGY, &energy.to_le_bytes(), same);
        push_command(&mut out, opcode::MODE, &[self.params.image_mode], same);
        push_command(&mut out, opcode::SPEED, &[self.params.print_speed], same);
        for row in bitmap.rows() {
            // The wire packs the leftmost dot in the lowest bit.
            push_command(&mut out, opcode::RASTER_ROW, row, u8::reverse_bits);
        }
        if steps > 0 {
            push_command(&mut out, opcode::SPEED, &[self.params.feed_speed], same);
            let mut remaining = steps;
            while remaining > 0 {
                let amount = remaining.min(self.params.feed_command_max_steps);
                push_command(&mut out, opcode::FEED, &amount.to_le_bytes(), same);
                remaining -= amount;
            }
        }
        Ok(out)
    }
}

impl PrinterDriver for Iprint5178Driver {
    fn protocol_id(&self) -> &'static str {
        PROTOCOL_ID
    }

    fn protocol_version(&self) -> u32 {
        PROTOCOL_VERSION
    }

    fn new_decoder(&self) -> Box<dyn NotificationDecoder> {
        Box::new(Iprint5178Decoder { frames: FrameDecoder::new() })
    }

    fn status_query(&self) -> Option<Vec<u8>> {
        Some(command(opcode::STATUS, &[0x00]))
    }

    fn status_timeout(&self) -> Duration {
        Duration::from_millis(self.params.timing.status_timeout_ms)
    }

    fn diagnostic_queries(&self) -> Vec<DiagnosticQuery> {
        vec![DiagnosticQuery { name: "device_info", bytes: command(opcode::DEVICE_INFO, &[0x00]), required: false }]
    }

    fn densities(&self) -> Vec<Density> {
        self.params.energy.keys().copied().collect()
    }

    fn feed(&self) -> FeedCapability {
        self.params.feed
    }

    fn width_dots(&self) -> u32 {
        self.params.width_dots
    }

    fn max_height(&self) -> u32 {
        self.params.max_height
    }

    fn plan_job(&self, request: &PrintRequest<'_>) -> Result<ExchangePlan, ProtocolError> {
        let bytes = self.encode_job(request)?;
        let t = &self.params.timing;
        Ok(ExchangePlan {
            steps: vec![
                PlanStep::RequireReady { timeout: self.status_timeout() },
                PlanStep::SendPrintData { bytes },
                PlanStep::Settle(Duration::from_millis(t.settle_ms)),
                PlanStep::PollUntilReady {
                    interval: Duration::from_millis(t.ready_poll_ms),
                    deadline: Duration::from_millis(t.ready_deadline_ms),
                    query_timeout: self.status_timeout(),
                },
            ],
            completion: CompletionPolicy::StatusConfirmedReady,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;
    use crate::MonoBitmap;

    fn driver() -> Iprint5178Driver {
        Iprint5178Driver::new(Iprint5178Params::sc03h_reference()).unwrap()
    }

    fn response(op: u8, payload: &[u8]) -> Vec<u8> {
        frame_with_direction(op, DIRECTION_RESPONSE, payload).unwrap()
    }

    #[test]
    fn crc_check_vector() {
        assert_eq!(crc8(b"123456789"), 0xF4);
    }

    #[test]
    fn golden_commands() {
        assert_eq!(hex::encode(command(0xA3, &[0])), "5178a30001000000ff");
        assert_eq!(hex::encode(command(0xA4, &[0x33])), "5178a40001003399ff");
        assert_eq!(hex::encode(command(0xA1, &[0x30, 0])), "5178a10002003000f9ff");
        assert_eq!(hex::encode(command(0xBE, &[1])), "5178be0001000107ff");
    }

    #[test]
    fn real_status_reply() {
        let frames = FrameDecoder::new().feed(&hex::decode("5178a301030000122788ff").unwrap());
        assert_eq!(frames, vec![Frame { opcode: 0xA3, direction: 1, payload: vec![0, 0x12, 0x27] }]);
    }

    #[test]
    fn fragmentation_noise_concatenation_and_crc_recovery() {
        let a = response(0xAE, &[0x10]);
        let b = response(0xAE, &[0x00]);
        let mut corrupt = a.clone();
        let n = corrupt.len();
        corrupt[n - 2] ^= 1;
        let mut d = FrameDecoder::new();
        let mut first = b"garbage".to_vec();
        first.push(a[0]);
        assert!(d.feed(&first).is_empty());
        assert!(d.feed(&a[1..7]).is_empty());
        let mut rest = a[7..].to_vec();
        rest.extend(&corrupt);
        rest.extend(&b);
        let payloads: Vec<_> = d.feed(&rest).into_iter().map(|f| f.payload).collect();
        assert_eq!(payloads, vec![vec![0x10], vec![0x00]]);
    }

    #[test]
    fn oversize_header_is_skipped() {
        let mut data = hex::decode("5178a301ffff").unwrap();
        data.extend(response(0xA3, &[0]));
        assert_eq!(FrameDecoder::new().feed(&data).len(), 1);
    }

    #[test]
    fn decoder_events_ignore_commands_and_parse_status() {
        let d = driver();
        let mut dec = d.new_decoder();
        let mut data = command(0xA3, &[0x01]); // our own echo/command direction: ignored
        data.extend(response(0xA3, &[0x05, 0, 0]));
        data.extend(response(0xAE, &[0x10]));
        data.extend(response(0xAE, &[0x00]));
        data.extend(hex::decode("5178a8010a00810003332e302e34390015ff").unwrap());
        let events = dec.feed(&data);
        assert_eq!(events.len(), 4);
        let ProtocolEvent::Status(report) = &events[0] else { panic!() };
        assert_eq!(report.flags, vec![StatusFlag::PaperOut, StatusFlag::Overheated]);
        assert_eq!(report.text(), "Out of paper, Print head too hot");
        assert!(!report.is_ready());
        assert_eq!(events[1], ProtocolEvent::FlowPause);
        assert_eq!(events[2], ProtocolEvent::FlowResume);
        let ProtocolEvent::DeviceInfo(info) = &events[3] else { panic!() };
        assert_eq!(info.firmware.as_deref(), Some("3.0.49"));
    }

    #[test]
    fn lsb_first_black_wire_rows() {
        let bitmap = MonoBitmap::from_fn(384, 2, |x, y| matches!((x, y), (0, 0) | (7, 1) | (383, 1))).unwrap();
        let rows = driver()
            .wire_rows(&PrintRequest { bitmap: &bitmap, density: Density::Normal, trailing_feed_mm: 12 })
            .unwrap();
        let mut second = vec![0x80];
        second.extend([0; 46]);
        second.push(0x80);
        let mut first = vec![0x01];
        first.extend([0; 47]);
        assert_eq!(rows, vec![first, second]);
    }

    #[test]
    fn rejects_wrong_dimensions_and_bad_feed() {
        let d = driver();
        for (w, h) in [(380, 2), (384, 4001)] {
            let bitmap = MonoBitmap::new(w, h).unwrap();
            let req = PrintRequest { bitmap: &bitmap, density: Density::Normal, trailing_feed_mm: 12 };
            assert_matches!(d.encode_job(&req), Err(ProtocolError::InvalidRaster(_)));
        }
        let bitmap = MonoBitmap::new(384, 1).unwrap();
        let req = PrintRequest { bitmap: &bitmap, density: Density::Normal, trailing_feed_mm: 13 };
        assert_eq!(d.encode_job(&req), Err(ProtocolError::FeedOutOfRange { min: 0, max: 12 }));
    }

    #[test]
    fn plan_requires_fresh_status_and_confirms_ready() {
        let bitmap = MonoBitmap::new(384, 3).unwrap();
        let plan = driver()
            .plan_job(&PrintRequest { bitmap: &bitmap, density: Density::Normal, trailing_feed_mm: 0 })
            .unwrap();
        assert_matches!(plan.steps[0], PlanStep::RequireReady { .. });
        assert_matches!(plan.steps.last(), Some(PlanStep::PollUntilReady { .. }));
        assert_eq!(plan.completion, CompletionPolicy::StatusConfirmedReady);
        assert_eq!(plan.print_bytes(), 9 + 10 + 9 + 9 + 3 * 56); // A4, AF(u16), BE, BD, rows
    }
}
