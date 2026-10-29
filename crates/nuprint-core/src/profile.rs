//! Versioned model profiles and the registry that resolves them (docs/printer-support.md).
//!
//! Profiles are reviewed JSON bundled at build time. They hold data only — no executable
//! logic. A profile names a built-in protocol driver and supplies its parameters.

use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError, RwLock};

use nuprint_protocol::bitmap::MAX_PIXELS;
use nuprint_protocol::iprint::{self, Iprint5178Driver, Iprint5178Params};
use nuprint_protocol::{Density, PrinterDriver};
use nuprint_render::{MAX_TARGET_WIDTH, MIN_TARGET_WIDTH};
use nuprint_transport::{Advertisement, Endpoint, ServiceInfo, Uuid, WriteMode};
use serde::{Deserialize, Serialize};

use crate::api::{ProfileMatch, ProfileSummary, SupportLevel};
use crate::error::{AppError, AppResult, ErrorCode};

pub const SCHEMA_VERSION: u32 = 1;

macro_rules! builtin_profiles {
    ($($id:literal),* $(,)?) => {
        &[$(include_str!(concat!("../../../profiles/", $id, ".json"))),*]
    };
}

/// Profiles compiled into this build, by file name under `profiles/`. The verified SC03h comes
/// first; the rest are experimental models whose APK settings match it (docs/apk-printer-models.md).
pub const BUILTIN_PROFILES: &[&str] = builtin_profiles![
    "sc03h-iprint-384",
    "0019b-c-iprint-384",
    "click-print-iprint-384",
    "click-sound-iprint-384",
    "d1-iprint-384",
    "dl-ge225-iprint-384",
    "dt1-0-iprint-384",
    "dt1-r-iprint-384",
    "dtr-r0-iprint-384",
    "dy03-iprint-384",
    "emx-040256-iprint-384",
    "gb03pl-iprint-384",
    "gb03sl-iprint-384",
    "ht0125-iprint-384",
    "luxorp-px10-iprint-384",
    "ml-mp-01-iprint-384",
    "mtpr26bk-iprint-384",
    "mvmt-ink-iprint-384",
    "p1-iprint-384",
    "p2-iprint-384",
    "rossmann-iprint-384",
    "rt034h-iprint-384",
    "sc04h-iprint-384",
    "seznikecho-iprint-384",
    "tcm690464-iprint-384",
    "td-11308-iprint-384",
    "uxportmip-iprint-384",
    "x100-iprint-384",
    "x103h-iprint-384",
    "x18-iprint-384",
    "x2h-iprint-384",
    "x5h-iprint-384",
    "x6h-iprint-384",
    "x7h-iprint-384",
    "xc9-iprint-384",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtocolRef {
    pub id: String,
    pub version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatchRules {
    /// Case-sensitive advertised-name prefixes.
    pub name_prefixes: Vec<String>,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pacing {
    pub chunk_delay_ms: u64,
    pub boundary_bytes: usize,
    pub boundary_delay_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Timeouts {
    pub find_ms: u64,
    pub connect_ms: u64,
    pub subscribe_ms: u64,
    pub write_ms: u64,
    pub flow_pause_ms: u64,
    pub disconnect_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportKindRef {
    BleGatt,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransportSpec {
    pub kind: TransportKindRef,
    /// Any one of these services must contain the write characteristic.
    pub services: Vec<Uuid>,
    pub write_characteristic: Uuid,
    pub write_mode: WriteMode,
    pub notify_characteristics: Vec<Uuid>,
    pub min_notify_subscriptions: usize,
    /// Tested model ceiling; the transport's negotiated limit may lower it.
    pub max_chunk_bytes: usize,
    pub pacing: Pacing,
    pub timeouts: Timeouts,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RasterSpec {
    pub width_dots: u32,
    pub dots_per_mm: f32,
    pub max_height: u32,
    pub wire_packing: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaSpec {
    pub kind: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub max_copies: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Verification {
    /// `windows`, `macos` or `linux` (std::env::consts::OS values).
    pub os: String,
    pub level: SupportLevel,
    pub date: String,
    #[serde(default)]
    pub firmware: Option<String>,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub kind: String,
    pub reference: String,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelProfile {
    pub schema_version: u32,
    pub id: String,
    pub revision: u32,
    pub display_name: String,
    pub vendor: String,
    pub model: String,
    pub protocol: ProtocolRef,
    #[serde(rename = "match")]
    pub matching: MatchRules,
    pub transport: TransportSpec,
    pub raster: RasterSpec,
    pub media: MediaSpec,
    pub limits: Limits,
    /// Protocol-specific parameters, validated by the named driver.
    pub driver: serde_json::Value,
    pub default_density: Density,
    #[serde(default)]
    pub verification: Vec<Verification>,
    #[serde(default)]
    pub evidence: Vec<Evidence>,
    /// Set only on a profile edited on this computer: the built-in profile it was copied from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub derived_from: Option<String>,
}

/// The parts of a profile a user may edit on this computer. Identity (`id`, `protocol`,
/// `match`, `vendor`, `model`) and evidence stay with the built-in profile.
pub const TUNABLE_KEYS: [&str; 7] =
    ["display_name", "transport", "raster", "media", "limits", "driver", "default_density"];

/// Largest BLE write ever worth attempting; the ATT limit is 512 bytes.
const MAX_CUSTOM_CHUNK_BYTES: usize = 512;
/// The default ATT payload; anything smaller only crawls.
const MIN_CUSTOM_CHUNK_BYTES: usize = 20;
/// BLE connects and GATT operations cannot finish faster than this.
const MIN_CUSTOM_TIMEOUT_MS: u64 = 500;
const MAX_CUSTOM_NAME_CHARS: usize = 60;
const MIN_CUSTOM_DOTS_PER_MM: f32 = 1.0;
const MAX_CUSTOM_DOTS_PER_MM: f32 = 50.0;
/// Rows per page; the built-in profiles use 4000.
const MAX_CUSTOM_HEIGHT: u32 = 20_000;
const MAX_CUSTOM_COPIES: u16 = 100;
/// Longest pause between writes or for any timeout a custom profile may set.
const MAX_CUSTOM_WAIT_MS: u64 = 120_000;

/// Resolved endpoints on a connected device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoundEndpoints {
    pub write: Endpoint,
    pub write_mode: WriteMode,
}

impl ModelProfile {
    /// Parse and validate a profile document.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::Internal`] when the JSON is not a profile (unknown or missing fields) or fails
    /// [`Self::validate`].
    pub fn parse(json: &str) -> AppResult<Self> {
        let profile: ModelProfile = serde_json::from_str(json)
            .map_err(|e| AppError::new(ErrorCode::Internal, "Printer profile is invalid").with_detail(e.to_string()))?;
        profile.validate()?;
        Ok(profile)
    }

    fn invalid(&self, why: &str) -> AppError {
        AppError::new(ErrorCode::Internal, format!("Printer profile {} is invalid", self.id))
            .with_detail(why.to_owned())
    }

    /// Check that a profile is internally consistent and that its driver accepts it.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::Internal`], with the first problem in `detail`: an unsupported schema version, a
    /// missing ID, revision or name prefix, inconsistent transport endpoints or chunk ceiling, raster
    /// dimensions that are zero or beyond what can be rendered, no copies allowed, driver
    /// parameters the driver rejects or that disagree with the raster, or a default density the
    /// driver does not offer.
    pub fn validate(&self) -> AppResult<()> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(self.invalid("unsupported schema_version"));
        }
        if self.id.is_empty() || self.revision == 0 || self.matching.name_prefixes.iter().any(String::is_empty) {
            return Err(self.invalid("id, revision and non-empty name prefixes are required"));
        }
        if self.matching.name_prefixes.is_empty() {
            return Err(self.invalid("at least one name prefix is required"));
        }
        let t = &self.transport;
        if t.services.is_empty()
            || t.max_chunk_bytes == 0
            || t.min_notify_subscriptions > t.notify_characteristics.len()
        {
            return Err(self.invalid("transport endpoints or chunk ceiling are inconsistent"));
        }
        if self.raster.width_dots == 0 || self.raster.max_height == 0 || self.raster.dots_per_mm <= 0.0 {
            return Err(self.invalid("raster dimensions must be positive"));
        }
        // A page the renderer or the bitmap type would refuse can never print.
        let (width, height) = (self.raster.width_dots, self.raster.max_height);
        if !(MIN_TARGET_WIDTH..=MAX_TARGET_WIDTH).contains(&width) {
            return Err(self.invalid(&format!("width_dots must be {MIN_TARGET_WIDTH}–{MAX_TARGET_WIDTH}")));
        }
        if u64::from(width) * u64::from(height) > MAX_PIXELS {
            return Err(self.invalid(&format!("width_dots × max_height must be at most {MAX_PIXELS} dots")));
        }
        if self.limits.max_copies == 0 {
            return Err(self.invalid("max_copies must be at least 1"));
        }
        let driver = self.driver().map_err(|e| self.invalid(&e))?;
        if driver.width_dots() != self.raster.width_dots || driver.max_height() != self.raster.max_height {
            return Err(self.invalid("driver and raster dimensions disagree"));
        }
        if !driver.densities().contains(&self.default_density) {
            return Err(self.invalid("default density is not offered by the driver"));
        }
        Ok(())
    }

    /// The editable document: [`TUNABLE_KEYS`] of this profile as a JSON object.
    pub fn tunable_doc(&self) -> serde_json::Value {
        let full = serde_json::to_value(self).unwrap_or_default();
        let serde_json::Value::Object(mut map) = full else { return serde_json::Value::Null };
        map.retain(|key, _| TUNABLE_KEYS.contains(&key.as_str()));
        serde_json::Value::Object(map)
    }

    /// A user-edited copy of `base`: the keys in `doc` replace the base's, identity stays
    /// the base's, and the result must pass the same validation as a built-in profile plus
    /// sanity limits on values that would otherwise stall or flood the link.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::InvalidInput`] when `doc` is not an object, names a key outside the
    /// editable set, or produces values that fail validation or the edit limits, and
    /// [`ErrorCode::Internal`] if `base` itself cannot be serialized.
    pub fn customized(base: &ModelProfile, doc: &serde_json::Value, id: String, revision: u32) -> AppResult<Self> {
        let invalid = |msg: String| AppError::new(ErrorCode::InvalidInput, msg);
        let serde_json::Value::Object(edits) = doc else {
            return Err(invalid("The profile settings must be a JSON object.".into()));
        };
        if let Some(key) = edits.keys().find(|k| !TUNABLE_KEYS.contains(&k.as_str())) {
            return Err(invalid(format!(
                "\"{key}\" cannot be changed. Editable settings: {}.",
                TUNABLE_KEYS.join(", ")
            )));
        }
        let serde_json::Value::Object(mut merged) = serde_json::to_value(base).map_err(AppError::internal)? else {
            return Err(AppError::internal("profile did not serialize to an object"));
        };
        merged.extend(edits.clone());
        let mut profile: ModelProfile = serde_json::from_value(serde_json::Value::Object(merged))
            .map_err(|e| invalid(format!("These settings are not valid: {e}")))?;
        profile.id = id;
        profile.revision = revision;
        profile.derived_from = Some(base.id.clone());
        profile.verification.clear();
        profile.evidence.clear();
        profile.validate().map_err(|e| invalid(e.detail.clone().unwrap_or(e.message)))?;
        profile.check_custom_limits(base).map_err(invalid)?;
        if base.protocol.id == iprint::PROTOCOL_ID {
            let parse = |profile: &ModelProfile| {
                serde_json::from_value::<Iprint5178Params>(profile.driver.clone()).map_err(|e| e.to_string())
            };
            parse(&profile).and_then(|edited| edited.check_edit(&parse(base)?)).map_err(invalid)?;
        }
        Ok(profile)
    }

    /// Limits on edited values that cannot work whatever the printer is; see
    /// `Iprint5178Params::check_edit` for the protocol's own.
    fn check_custom_limits(&self, base: &ModelProfile) -> Result<(), String> {
        let name_len = self.display_name.trim().chars().count();
        if !(1..=MAX_CUSTOM_NAME_CHARS).contains(&name_len) || self.display_name.chars().any(char::is_control) {
            return Err(format!(
                "display_name must be 1–{MAX_CUSTOM_NAME_CHARS} characters without control characters."
            ));
        }
        let r = &self.raster;
        if !(MIN_CUSTOM_DOTS_PER_MM..=MAX_CUSTOM_DOTS_PER_MM).contains(&r.dots_per_mm)
            || r.max_height > MAX_CUSTOM_HEIGHT
        {
            return Err(format!(
                "dots_per_mm must be {MIN_CUSTOM_DOTS_PER_MM}–{MAX_CUSTOM_DOTS_PER_MM} and max_height at most {MAX_CUSTOM_HEIGHT}."
            ));
        }
        if r.wire_packing != base.raster.wire_packing {
            return Err("wire_packing describes how the driver packs rows and cannot be changed.".into());
        }
        if self.limits.max_copies > MAX_CUSTOM_COPIES {
            return Err(format!("max_copies must be at most {MAX_CUSTOM_COPIES}."));
        }
        let t = &self.transport;
        if t.min_notify_subscriptions == 0 {
            return Err(
                "min_notify_subscriptions must be at least 1: without status notifications nothing can print.".into()
            );
        }
        if !(MIN_CUSTOM_CHUNK_BYTES..=MAX_CUSTOM_CHUNK_BYTES).contains(&t.max_chunk_bytes) {
            return Err(format!("max_chunk_bytes must be {MIN_CUSTOM_CHUNK_BYTES}–{MAX_CUSTOM_CHUNK_BYTES}."));
        }
        let waits = [
            ("chunk_delay_ms", t.pacing.chunk_delay_ms),
            ("boundary_delay_ms", t.pacing.boundary_delay_ms),
            ("find_ms", t.timeouts.find_ms),
            ("connect_ms", t.timeouts.connect_ms),
            ("subscribe_ms", t.timeouts.subscribe_ms),
            ("write_ms", t.timeouts.write_ms),
            ("flow_pause_ms", t.timeouts.flow_pause_ms),
            ("disconnect_ms", t.timeouts.disconnect_ms),
        ];
        if let Some((name, _)) = waits.iter().find(|(_, ms)| *ms > MAX_CUSTOM_WAIT_MS) {
            return Err(format!("{name} must be at most {MAX_CUSTOM_WAIT_MS} ms."));
        }
        let timeouts = [
            ("find_ms", t.timeouts.find_ms),
            ("connect_ms", t.timeouts.connect_ms),
            ("subscribe_ms", t.timeouts.subscribe_ms),
            ("write_ms", t.timeouts.write_ms),
            ("flow_pause_ms", t.timeouts.flow_pause_ms),
            ("disconnect_ms", t.timeouts.disconnect_ms),
        ];
        if let Some((name, _)) = timeouts.iter().find(|(_, ms)| *ms < MIN_CUSTOM_TIMEOUT_MS) {
            return Err(format!("{name} must be at least {MIN_CUSTOM_TIMEOUT_MS} ms."));
        }
        Ok(())
    }

    /// Instantiate the built-in driver named by `protocol.id`.
    ///
    /// # Errors
    ///
    /// A message when there is no built-in driver for the profile's protocol and version, or the
    /// driver rejects its `driver` parameters.
    pub fn driver(&self) -> Result<Arc<dyn PrinterDriver>, String> {
        match self.protocol.id.as_str() {
            iprint::PROTOCOL_ID if self.protocol.version == iprint::PROTOCOL_VERSION => {
                let params: Iprint5178Params =
                    serde_json::from_value(self.driver.clone()).map_err(|e| e.to_string())?;
                Ok(Arc::new(Iprint5178Driver::new(params)?))
            }
            other => Err(format!("no built-in driver for protocol {other} v{}", self.protocol.version)),
        }
    }

    /// Case-sensitive name match. A match is a candidate, never permission to print.
    pub fn matches_name(&self, name: &str) -> bool {
        self.matching.name_prefixes.iter().any(|p| name.starts_with(p.as_str()))
    }

    pub fn support_on(&self, os: &str) -> (SupportLevel, String) {
        if self.derived_from.is_some() {
            return (
                SupportLevel::Experimental,
                "Settings edited on this computer. They have not been verified on a real printer.".to_owned(),
            );
        }
        match self.verification.iter().filter(|v| v.os == os).max_by_key(|v| v.level) {
            Some(v) => (v.level, format!("{} ({})", v.summary, v.date)),
            None => {
                let tested: Vec<_> = self.verification.iter().map(|v| v.os.as_str()).collect();
                let note = if tested.is_empty() {
                    "No physical verification recorded for this profile.".to_owned()
                } else {
                    format!("Verified on {} only; untested on {os}. Printing here is experimental.", tested.join(", "))
                };
                (SupportLevel::Experimental, note)
            }
        }
    }

    /// Check required endpoints on a connected device without writing anything.
    ///
    /// # Errors
    ///
    /// A message naming the services when none of the profile's services has its write
    /// characteristic with a property that can be written.
    pub fn bind(&self, services: &[ServiceInfo]) -> Result<BoundEndpoints, String> {
        let t = &self.transport;
        for service in services.iter().filter(|s| t.services.contains(&s.uuid)) {
            if let Some(c) = service.characteristic(t.write_characteristic) {
                let mode = match t.write_mode {
                    WriteMode::WithoutResponse if c.properties.write_without_response => WriteMode::WithoutResponse,
                    _ if c.properties.write => WriteMode::WithResponse,
                    _ if c.properties.write_without_response => WriteMode::WithoutResponse,
                    _ => continue,
                };
                return Ok(BoundEndpoints {
                    write: Endpoint { service: service.uuid, characteristic: c.uuid },
                    write_mode: mode,
                });
            }
        }
        let names: Vec<String> = t.services.iter().map(Uuid::to_string).collect();
        Err(format!(
            "print service ({}) with write characteristic {} was not found",
            names.join(" or "),
            t.write_characteristic
        ))
    }

    /// Notify endpoints in this profile's services that the device actually offers.
    pub fn notify_endpoints(&self, services: &[ServiceInfo]) -> Vec<Endpoint> {
        let t = &self.transport;
        services
            .iter()
            .filter(|s| t.services.contains(&s.uuid))
            .flat_map(|s| {
                s.characteristics
                    .iter()
                    .filter(|c| t.notify_characteristics.contains(&c.uuid))
                    .filter(|c| c.properties.notify || c.properties.indicate)
                    .map(move |c| Endpoint { service: s.uuid, characteristic: c.uuid })
            })
            .collect()
    }
}

#[derive(Clone)]
pub struct ResolvedProfile {
    pub profile: Arc<ModelProfile>,
    pub driver: Arc<dyn PrinterDriver>,
}

impl ResolvedProfile {
    pub fn summary(&self, os: &str) -> ProfileSummary {
        let p = &self.profile;
        let (support, support_note) = p.support_on(os);
        ProfileSummary {
            id: p.id.clone(),
            revision: p.revision,
            display_name: p.display_name.clone(),
            vendor: p.vendor.clone(),
            model: p.model.clone(),
            protocol_id: p.protocol.id.clone(),
            support,
            support_note,
            width_dots: p.raster.width_dots,
            dots_per_mm: p.raster.dots_per_mm,
            max_height: p.raster.max_height,
            densities: self.driver.densities(),
            default_density: p.default_density,
            feed: self.driver.feed(),
            max_copies: p.limits.max_copies,
            media: p.media.description.clone(),
            completion: nuprint_protocol::CompletionPolicy::StatusConfirmedReady,
        }
    }

    /// Reject darkness, extra feed or copies this printer does not accept.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::InvalidInput`] when the density is not offered, the feed is outside the
    /// driver's range, or `copies` is zero or above the profile's limit.
    pub fn check_job_options(&self, density: Density, feed_mm: u16, copies: Option<u16>) -> AppResult<()> {
        if !self.driver.densities().contains(&density) {
            return Err(AppError::invalid("That darkness setting is not available for this printer."));
        }
        let feed = self.driver.feed();
        if !feed.contains(feed_mm) {
            return Err(AppError::invalid(format!(
                "Extra feed must be a whole number from {} to {} mm",
                feed.min_mm, feed.max_mm
            )));
        }
        let max_copies = self.profile.limits.max_copies;
        if copies.is_some_and(|c| !(1..=max_copies).contains(&c)) {
            return Err(AppError::invalid(format!("Copies must be from 1 to {max_copies}.")));
        }
        Ok(())
    }

    pub fn as_match(&self, os: &str) -> ProfileMatch {
        ProfileMatch {
            profile_id: self.profile.id.clone(),
            revision: self.profile.revision,
            display_name: self.profile.display_name.clone(),
            support: self.profile.support_on(os).0,
        }
    }
}

pub struct ProfileRegistry {
    profiles: BTreeMap<String, ResolvedProfile>,
    /// Profiles edited on this computer, every revision, because queued jobs name the exact
    /// revision they were rendered for. Never offered by discovery.
    custom: RwLock<BTreeMap<(String, u32), ResolvedProfile>>,
    os: String,
}

impl ProfileRegistry {
    /// The registry of the profiles embedded at build time.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::Internal`] when an embedded profile is invalid, two share an ID, or two name
    /// prefixes overlap. That is a broken build, which the tests catch.
    pub fn builtin() -> AppResult<Self> {
        Self::from_json(BUILTIN_PROFILES.iter().copied(), std::env::consts::OS)
    }

    /// A registry built from profile documents, for tests and the built-in set.
    ///
    /// # Errors
    ///
    /// As [`Self::builtin`], for the given documents.
    pub fn from_json<'a>(sources: impl IntoIterator<Item = &'a str>, os: &str) -> AppResult<Self> {
        let mut profiles = BTreeMap::new();
        for json in sources {
            let profile = ModelProfile::parse(json)?;
            let driver = profile.driver().map_err(AppError::internal)?;
            let id = profile.id.clone();
            if profiles.insert(id.clone(), ResolvedProfile { profile: Arc::new(profile), driver }).is_some() {
                return Err(AppError::internal(format!("duplicate profile id {id}")));
            }
        }
        reject_overlapping_prefixes(&profiles)?;
        Ok(Self { profiles, custom: RwLock::default(), os: os.to_owned() })
    }

    pub fn os(&self) -> &str {
        &self.os
    }

    /// The built-in profile, or the newest revision of a custom one.
    pub fn get(&self, id: &str) -> Option<ResolvedProfile> {
        self.profiles.get(id).cloned().or_else(|| {
            let custom = self.custom.read().unwrap_or_else(PoisonError::into_inner);
            custom.range((id.to_owned(), 0)..=(id.to_owned(), u32::MAX)).next_back().map(|(_, p)| p.clone())
        })
    }

    /// Exact revision lookup for queued jobs; a different revision is never substituted.
    ///
    /// A profile is identified by `(id, revision)`, and a published revision never changes:
    /// editing a custom profile publishes the next revision, and a job keeps printing with the
    /// one it was planned for. [`Self::get`] returns the newest revision instead.
    ///
    /// ```
    /// use nuprint_core::profile::{ModelProfile, ProfileRegistry};
    ///
    /// let registry = ProfileRegistry::builtin()?;
    /// let base = registry.get("sc03h-iprint-384").expect("built-in profile");
    /// let edit = |speed: u8| {
    ///     let mut doc = base.profile.tunable_doc();
    ///     doc["driver"]["print_speed"] = speed.into();
    ///     ModelProfile::customized(&base.profile, &doc, "custom:p".into(), 1)
    /// };
    ///
    /// registry.add_custom(edit(11)?)?;
    /// // Revision 1 is taken for good; a second edit under it is refused, not substituted.
    /// assert!(registry.add_custom(edit(12)?).is_err());
    /// assert_eq!(registry.get_revision("custom:p", 1).unwrap().profile.driver["print_speed"], 11);
    /// assert!(registry.get_revision("custom:p", 2).is_none());
    /// assert_eq!(registry.next_custom_revision("custom:p"), 2);
    ///
    /// // A built-in profile answers only for its own revision.
    /// let built_in = base.profile.revision;
    /// assert!(registry.get_revision("sc03h-iprint-384", built_in).is_some());
    /// assert!(registry.get_revision("sc03h-iprint-384", built_in + 1).is_none());
    /// # Ok::<(), nuprint_core::AppError>(())
    /// ```
    pub fn get_revision(&self, id: &str, revision: u32) -> Option<ResolvedProfile> {
        if let Some(p) = self.profiles.get(id) {
            return (p.profile.revision == revision).then(|| p.clone());
        }
        self.custom.read().unwrap_or_else(PoisonError::into_inner).get(&(id.to_owned(), revision)).cloned()
    }

    /// Built-in profiles only; custom ones never take part in discovery.
    pub fn all(&self) -> impl Iterator<Item = &ResolvedProfile> {
        self.profiles.values()
    }

    /// Add a profile edited on this computer. It must name a built-in profile it derives from.
    ///
    /// # Errors
    ///
    /// The errors of [`Self::prepare_custom`] and [`Self::publish_custom`].
    pub fn add_custom(&self, profile: ModelProfile) -> AppResult<ResolvedProfile> {
        let resolved = self.prepare_custom(profile)?;
        self.publish_custom(resolved.clone())?;
        Ok(resolved)
    }

    /// Check an edited profile and build its driver without making it visible, so it can be
    /// stored first and published with [`Self::publish_custom`] only once that succeeded.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::Internal`] when the profile is invalid, has no `derived_from`, does not
    /// derive from a built-in profile, or reuses a built-in profile's ID.
    pub fn prepare_custom(&self, profile: ModelProfile) -> AppResult<ResolvedProfile> {
        profile.validate()?;
        let derived =
            profile.derived_from.as_deref().ok_or_else(|| AppError::internal("custom profile has no base"))?;
        if !self.profiles.contains_key(derived) || self.profiles.contains_key(&profile.id) {
            return Err(AppError::internal("custom profile does not derive from a built-in profile"));
        }
        let driver = profile.driver().map_err(AppError::internal)?;
        Ok(ResolvedProfile { profile: Arc::new(profile), driver })
    }

    /// Make a prepared profile visible. A revision is immutable once published, because queued
    /// jobs print with exactly the revision they name, so publishing one twice is refused.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::Internal`] when `(id, revision)` is already published.
    pub fn publish_custom(&self, resolved: ResolvedProfile) -> AppResult<()> {
        let key = (resolved.profile.id.clone(), resolved.profile.revision);
        let mut custom = self.custom.write().unwrap_or_else(PoisonError::into_inner);
        if custom.contains_key(&key) {
            return Err(AppError::internal(format!("{} revision {} already exists", key.0, key.1)));
        }
        custom.insert(key, resolved);
        Ok(())
    }

    /// The next free revision for a custom profile ID.
    pub fn next_custom_revision(&self, id: &str) -> u32 {
        let custom = self.custom.read().unwrap_or_else(PoisonError::into_inner);
        custom.range((id.to_owned(), 0)..=(id.to_owned(), u32::MAX)).next_back().map_or(1, |((_, rev), _)| rev + 1)
    }

    /// Best-supported first, so the first entry is a sensible default when no printer is chosen.
    /// Custom profiles are listed too (newest revision), so saved printers can describe themselves.
    pub fn summaries(&self) -> Vec<ProfileSummary> {
        let mut out: Vec<_> = self.all().map(|p| p.summary(&self.os)).collect();
        {
            let custom = self.custom.read().unwrap_or_else(PoisonError::into_inner);
            let mut latest: BTreeMap<&str, &ResolvedProfile> = BTreeMap::new();
            for ((id, _), p) in custom.iter() {
                latest.insert(id.as_str(), p);
            }
            out.extend(latest.values().map(|p| p.summary(&self.os)));
        }
        out.sort_by(|a, b| b.support.cmp(&a.support).then_with(|| a.display_name.cmp(&b.display_name)));
        out
    }

    /// Name-based candidates for a discovery observation.
    pub fn candidates(&self, adv: &Advertisement) -> Vec<&ResolvedProfile> {
        let Some(name) = adv.local_name.as_deref() else { return Vec::new() };
        self.all().filter(|p| p.profile.matches_name(name)).collect()
    }
}

/// A device name must never match two profiles by prefix: "XC9-" would also claim "XC9-FL01-…".
fn reject_overlapping_prefixes(profiles: &BTreeMap<String, ResolvedProfile>) -> AppResult<()> {
    let prefixes: Vec<(&str, &str)> = profiles
        .values()
        .flat_map(|p| p.profile.matching.name_prefixes.iter().map(move |n| (p.profile.id.as_str(), n.as_str())))
        .collect();
    for (i, (id_a, a)) in prefixes.iter().enumerate() {
        for (id_b, b) in &prefixes[i + 1..] {
            if a.starts_with(b) || b.starts_with(a) {
                return Err(AppError::internal(format!("name prefix {a:?} ({id_a}) overlaps {b:?} ({id_b})")));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuprint_transport::{CharProps, CharacteristicInfo, short_uuid};

    fn registry() -> ProfileRegistry {
        ProfileRegistry::from_json(BUILTIN_PROFILES.iter().copied(), "windows").unwrap()
    }

    fn adv(name: &str) -> Advertisement {
        Advertisement {
            backend_id: "id".into(),
            local_name: Some(name.into()),
            address: None,
            rssi: None,
            services: vec![short_uuid(0xAF30)],
            manufacturer_ids: vec![],
        }
    }

    pub(crate) fn sc03h_services(write_props: CharProps, notify: bool) -> Vec<ServiceInfo> {
        let n = CharProps { notify, ..Default::default() };
        vec![ServiceInfo {
            uuid: short_uuid(0xAE30),
            characteristics: vec![
                CharacteristicInfo { uuid: short_uuid(0xAE01), properties: write_props },
                CharacteristicInfo { uuid: short_uuid(0xAE02), properties: n },
                CharacteristicInfo { uuid: short_uuid(0xAE04), properties: n },
            ],
        }]
    }

    #[test]
    fn builtin_profile_is_valid_and_matches_reference_driver() {
        let r = registry();
        let sc03h = r.get("sc03h-iprint-384").unwrap();
        let params: Iprint5178Params = serde_json::from_value(sc03h.profile.driver.clone()).unwrap();
        assert_eq!(params, Iprint5178Params::sc03h_reference());
        assert_eq!(sc03h.summary("windows").support, SupportLevel::VerifiedBasic);
        assert_eq!(sc03h.summary("macos").support, SupportLevel::VerifiedBasic);
        assert_eq!(sc03h.summary("linux").support, SupportLevel::Experimental);
    }

    #[test]
    fn experimental_profiles_copy_the_verified_sc03h_settings() {
        let r = registry();
        let sc03h = &r.get("sc03h-iprint-384").unwrap().profile;
        let others: Vec<_> = r.all().filter(|p| p.profile.id != sc03h.id).collect();
        assert_eq!(others.len(), BUILTIN_PROFILES.len() - 1);
        for p in others {
            let p = &p.profile;
            assert_eq!(p.driver, sc03h.driver, "{} driver differs from SC03h", p.id);
            assert_eq!(serde_json::to_value(&p.transport).unwrap(), serde_json::to_value(&sc03h.transport).unwrap());
            assert_eq!(serde_json::to_value(&p.raster).unwrap(), serde_json::to_value(&sc03h.raster).unwrap());
            assert!(p.verification.is_empty(), "{} claims hardware verification", p.id);
            for os in ["windows", "macos", "linux"] {
                assert_eq!(p.support_on(os).0, SupportLevel::Experimental, "{} on {os}", p.id);
            }
        }
    }

    #[test]
    fn every_profile_file_is_built_in() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../profiles");
        let files = std::fs::read_dir(dir)
            .unwrap()
            .filter(|e| e.as_ref().unwrap().path().extension().is_some_and(|x| x == "json"));
        assert_eq!(files.count(), BUILTIN_PROFILES.len());
    }

    #[test]
    fn every_builtin_profile_passes_the_limits_for_edited_profiles() {
        // The limits must never contradict reviewed values: re-saving a built-in profile unchanged works.
        let r = registry();
        for p in r.all() {
            let edited = ModelProfile::customized(&p.profile, &p.profile.tunable_doc(), "custom:x".into(), 1);
            assert!(edited.is_ok(), "{}: {:?}", p.profile.id, edited.err().map(|e| e.message));
        }
    }

    #[test]
    fn edited_sizes_and_status_waits_stay_within_what_can_print() {
        let r = registry();
        let base = &r.get("sc03h-iprint-384").unwrap().profile;
        let edit = |width: u32, height: u32, status_ms: u64| {
            let mut doc = base.tunable_doc();
            doc["raster"]["width_dots"] = width.into();
            doc["driver"]["width_dots"] = width.into();
            doc["raster"]["max_height"] = height.into();
            doc["driver"]["max_height"] = height.into();
            doc["driver"]["timing"]["status_timeout_ms"] = status_ms.into();
            ModelProfile::customized(base, &doc, "custom:p".into(), 1).map_err(|e| e.message)
        };
        assert!(edit(96, 4000, 3000).is_ok());
        assert!(edit(88, 4000, 3000).unwrap_err().contains("width_dots"));
        assert!(edit(8192, 8192, 3000).is_ok(), "exactly the largest bitmap");
        assert!(edit(8200, 4000, 3000).unwrap_err().contains("width_dots"));
        assert!(edit(8192, 8193, 3000).unwrap_err().contains("max_height"));
        assert!(edit(384, 4000, 60_000).is_ok());
        assert!(edit(384, 4000, 60_001).unwrap_err().contains("status_timeout_ms"));
        assert!(edit(384, 4000, u64::MAX).unwrap_err().contains("status_timeout_ms"));
    }

    #[test]
    fn overlapping_name_prefixes_are_rejected() {
        let base: serde_json::Value = serde_json::from_str(BUILTIN_PROFILES[0]).unwrap();
        let mut longer = base.clone();
        longer["id"] = "longer".into();
        longer["match"]["name_prefixes"] = serde_json::json!(["SC03h-X"]);
        let sources = [base.to_string(), longer.to_string()];
        assert!(ProfileRegistry::from_json(sources.iter().map(String::as_str), "windows").is_err());
    }

    #[test]
    fn verified_profile_is_listed_first() {
        let r = registry();
        assert_eq!(r.summaries()[0].id, "sc03h-iprint-384");
        assert_eq!(r.summaries().len(), BUILTIN_PROFILES.len());
    }

    #[test]
    fn lowercase_h_variants_do_not_match_uppercase_names() {
        let r = registry();
        assert_eq!(r.candidates(&adv("X6h-1A2B"))[0].profile.id, "x6h-iprint-384");
        assert!(r.candidates(&adv("X6H-1A2B")).is_empty());
        assert_eq!(r.candidates(&adv("XC9-FL01-1A2B"))[0].profile.id, "xc9-iprint-384");
        assert!(r.candidates(&adv("0019B-D-1A2B")).is_empty());
        assert!(r.candidates(&adv("Pocket Printer-1A2B")).is_empty());
    }

    #[test]
    fn lowercase_model_only() {
        let r = registry();
        assert_eq!(r.candidates(&adv("SC03h-3DE8")).len(), 1);
        assert!(r.candidates(&adv("SC03H-3DE8")).is_empty());
        assert!(r.candidates(&adv("sc03h-3DE8")).is_empty());
        assert!(r.candidates(&adv("GB01-1234")).is_empty());
        let unnamed = Advertisement { local_name: None, ..adv("x") };
        assert!(r.candidates(&unnamed).is_empty());
    }

    #[test]
    fn endpoint_binding_requires_write_and_notify() {
        let r = registry();
        let p = &r.get("sc03h-iprint-384").unwrap().profile;
        let wwr = CharProps { write_without_response: true, ..Default::default() };
        let bound = p.bind(&sc03h_services(wwr, true)).unwrap();
        assert_eq!(bound.write.characteristic, short_uuid(0xAE01));
        assert_eq!(bound.write_mode, WriteMode::WithoutResponse);
        assert_eq!(p.notify_endpoints(&sc03h_services(wwr, true)).len(), 2);
        assert!(p.notify_endpoints(&sc03h_services(wwr, false)).is_empty());
        assert!(p.bind(&sc03h_services(CharProps::default(), true)).is_err());
        // Same characteristic under an unrelated service is not accepted.
        let mut other = sc03h_services(wwr, true);
        other[0].uuid = short_uuid(0xFFF0);
        assert!(p.bind(&other).is_err());
    }

    #[test]
    fn invalid_profiles_are_rejected() {
        let base: serde_json::Value = serde_json::from_str(BUILTIN_PROFILES[0]).unwrap();
        let mutate = |f: &dyn Fn(&mut serde_json::Value)| {
            let mut v = base.clone();
            f(&mut v);
            ModelProfile::parse(&v.to_string())
        };
        assert!(mutate(&|v| v["unknown_field"] = 1.into()).is_err());
        assert!(mutate(&|v| v["raster"]["width_dots"] = 576.into()).is_err());
        assert!(mutate(&|v| v["protocol"]["id"] = "esc-pos".into()).is_err());
        assert!(mutate(&|v| v["match"]["name_prefixes"] = serde_json::json!([])).is_err());
        assert!(mutate(&|v| v["driver"]["feed"]["default_mm"] = 13.into()).is_err());
        assert!(mutate(&|v| v["schema_version"] = 2.into()).is_err());
    }

    #[test]
    fn a_published_revision_is_never_replaced() {
        let registry = registry();
        let base = registry.get("sc03h-iprint-384").unwrap();
        let edit = |speed: u8| {
            let mut doc = base.profile.tunable_doc();
            doc["driver"]["print_speed"] = speed.into();
            ModelProfile::customized(&base.profile, &doc, "custom:p".into(), 1).unwrap()
        };
        let prepared = registry.prepare_custom(edit(11)).unwrap();
        assert!(registry.get("custom:p").is_none(), "preparing publishes nothing");
        registry.publish_custom(prepared).unwrap();
        assert!(registry.add_custom(edit(12)).is_err());
        assert_eq!(registry.get_revision("custom:p", 1).unwrap().profile.driver["print_speed"], 11);
    }
}
