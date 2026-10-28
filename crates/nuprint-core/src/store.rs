//! SQLite is the authoritative queue/settings store (docs/technical-spec.md §3).
//!
//! Layout under the data directory:
//! `state.sqlite3` (jobs, pages, attempts, events, printers, preferences) and
//! `jobs/<job-id>/page-<n>.bin` (canonical packed rows, written atomically).

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use nuprint_protocol::{Density, MonoBitmap};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::api::JobState;
use crate::error::{AppError, AppResult, ErrorCode};

pub const SCHEMA_VERSION: i64 = 3;
pub const COMPLETED_METADATA_DAYS: i64 = 30;
pub const FAILED_CONTENT_DAYS: i64 = 7;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

const MIGRATION_1: &str = "
CREATE TABLE printers (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    backend_kind TEXT NOT NULL,
    backend_id TEXT NOT NULL,
    address TEXT,
    profile_id TEXT NOT NULL,
    profile_revision INTEGER NOT NULL,
    firmware TEXT,
    last_seen_ms INTEGER,
    density TEXT NOT NULL,
    feed_mm INTEGER NOT NULL,
    created_ms INTEGER NOT NULL
);
CREATE TABLE preferences (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE jobs (
    id TEXT PRIMARY KEY,
    idempotency_key TEXT NOT NULL UNIQUE,
    title TEXT NOT NULL,
    source_kind TEXT NOT NULL,
    printer_id TEXT NOT NULL,
    printer_name TEXT NOT NULL,
    backend_id TEXT NOT NULL,
    profile_id TEXT NOT NULL,
    profile_revision INTEGER NOT NULL,
    protocol_id TEXT NOT NULL,
    protocol_version INTEGER NOT NULL,
    state TEXT NOT NULL,
    created_ms INTEGER NOT NULL,
    updated_ms INTEGER NOT NULL,
    copies INTEGER NOT NULL,
    density TEXT NOT NULL,
    feed_mm INTEGER NOT NULL,
    dots_per_mm REAL NOT NULL,
    renderer_version TEXT NOT NULL,
    settings_json TEXT NOT NULL,
    content_retained INTEGER NOT NULL,
    bytes_total INTEGER NOT NULL,
    bytes_sent INTEGER NOT NULL,
    error_json TEXT,
    partial_output INTEGER NOT NULL,
    retry_of TEXT,
    revision INTEGER NOT NULL
);
CREATE INDEX jobs_state ON jobs(state, created_ms);
CREATE TABLE pages (
    job_id TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    idx INTEGER NOT NULL,
    source_page INTEGER NOT NULL,
    width INTEGER NOT NULL,
    height INTEGER NOT NULL,
    path TEXT NOT NULL,
    sha256 TEXT NOT NULL,
    encoded_bytes INTEGER NOT NULL,
    PRIMARY KEY (job_id, idx)
);
CREATE TABLE attempts (
    id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    started_ms INTEGER NOT NULL,
    ended_ms INTEGER,
    phase TEXT NOT NULL,
    page_cursor INTEGER NOT NULL,
    copy_cursor INTEGER NOT NULL,
    bytes_submitted INTEGER NOT NULL,
    last_status TEXT,
    outcome TEXT,
    error_code TEXT
);
CREATE TABLE events (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    attempt_id TEXT NOT NULL,
    at_ms INTEGER NOT NULL,
    kind TEXT NOT NULL,
    details TEXT NOT NULL
);
";

/// Printers saved with settings edited on this computer point `profile_id` at a custom profile
/// (see `custom_profiles`) and keep the built-in profile they came from in `base_profile_id`.
const MIGRATION_2: &str = "
    ALTER TABLE printers ADD COLUMN base_profile_id TEXT;
    CREATE TABLE custom_profiles (
        id TEXT NOT NULL,
        revision INTEGER NOT NULL,
        base_profile_id TEXT NOT NULL,
        json TEXT NOT NULL,
        created_ms INTEGER NOT NULL,
        PRIMARY KEY (id, revision)
    );
";

/// The preview of a finished job's first page, kept in its folder after the print data is gone.
const THUMBNAIL_FILE: &str = "thumbnail.png";

/// Jobs used to store the whole source, including typed text and PDF passwords. Keep only how a
/// job was made. SQLite may keep the old bytes in free pages and in the `state.v2.bak` backup.
const MIGRATION_3: &str = "
    UPDATE jobs SET settings_json = json_remove(settings_json, '$.text', '$.password')
    WHERE json_valid(settings_json);
";

#[derive(Debug, Clone, PartialEq)]
pub struct PrinterRow {
    pub id: String,
    pub name: String,
    pub backend_kind: String,
    pub backend_id: String,
    pub address: Option<String>,
    pub profile_id: String,
    pub profile_revision: u32,
    /// The built-in profile a custom `profile_id` was copied from; `None` for a built-in one.
    pub base_profile_id: Option<String>,
    pub firmware: Option<String>,
    pub last_seen_ms: Option<i64>,
    pub density: Density,
    pub feed_mm: u16,
    pub created_ms: i64,
}

/// One stored revision of a profile edited on this computer.
#[derive(Debug, Clone, PartialEq)]
pub struct CustomProfileRow {
    pub id: String,
    pub revision: u32,
    pub json: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct JobRow {
    pub id: String,
    pub idempotency_key: String,
    pub title: String,
    pub source_kind: String,
    pub printer_id: String,
    pub printer_name: String,
    pub backend_id: String,
    pub profile_id: String,
    pub profile_revision: u32,
    pub protocol_id: String,
    pub protocol_version: u32,
    pub state: JobState,
    pub created_ms: i64,
    pub updated_ms: i64,
    pub copies: u16,
    pub density: Density,
    pub feed_mm: u16,
    pub dots_per_mm: f32,
    pub renderer_version: String,
    pub settings_json: String,
    pub content_retained: bool,
    pub bytes_total: i64,
    pub bytes_sent: i64,
    pub error: Option<AppError>,
    pub partial_output: bool,
    pub retry_of: Option<String>,
    pub revision: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PageRow {
    pub job_id: String,
    pub idx: u32,
    pub source_page: u32,
    pub width: u32,
    pub height: u32,
    pub path: String,
    pub sha256: String,
    pub encoded_bytes: i64,
}

fn error_json(error: Option<&AppError>) -> Option<String> {
    error.map(|e| serde_json::to_string(e).unwrap_or_default())
}

fn parse_density(s: String) -> rusqlite::Result<Density> {
    s.parse().map_err(|_| rusqlite::Error::InvalidColumnType(0, "density".into(), rusqlite::types::Type::Text))
}

fn printer_from_row(r: &Row<'_>) -> rusqlite::Result<PrinterRow> {
    Ok(PrinterRow {
        id: r.get("id")?,
        name: r.get("name")?,
        backend_kind: r.get("backend_kind")?,
        backend_id: r.get("backend_id")?,
        address: r.get("address")?,
        profile_id: r.get("profile_id")?,
        profile_revision: r.get("profile_revision")?,
        base_profile_id: r.get("base_profile_id")?,
        firmware: r.get("firmware")?,
        last_seen_ms: r.get("last_seen_ms")?,
        density: parse_density(r.get("density")?)?,
        feed_mm: r.get("feed_mm")?,
        created_ms: r.get("created_ms")?,
    })
}

fn job_from_row(r: &Row<'_>) -> rusqlite::Result<JobRow> {
    let state: String = r.get("state")?;
    let error_json: Option<String> = r.get("error_json")?;
    Ok(JobRow {
        id: r.get("id")?,
        idempotency_key: r.get("idempotency_key")?,
        title: r.get("title")?,
        source_kind: r.get("source_kind")?,
        printer_id: r.get("printer_id")?,
        printer_name: r.get("printer_name")?,
        backend_id: r.get("backend_id")?,
        profile_id: r.get("profile_id")?,
        profile_revision: r.get("profile_revision")?,
        protocol_id: r.get("protocol_id")?,
        protocol_version: r.get("protocol_version")?,
        state: JobState::parse(&state)
            .ok_or_else(|| rusqlite::Error::InvalidColumnType(0, "state".into(), rusqlite::types::Type::Text))?,
        created_ms: r.get("created_ms")?,
        updated_ms: r.get("updated_ms")?,
        copies: r.get("copies")?,
        density: parse_density(r.get("density")?)?,
        feed_mm: r.get("feed_mm")?,
        dots_per_mm: r.get("dots_per_mm")?,
        renderer_version: r.get("renderer_version")?,
        settings_json: r.get("settings_json")?,
        content_retained: r.get("content_retained")?,
        bytes_total: r.get("bytes_total")?,
        bytes_sent: r.get("bytes_sent")?,
        error: error_json.and_then(|j| serde_json::from_str(&j).ok()),
        partial_output: r.get("partial_output")?,
        retry_of: r.get("retry_of")?,
        revision: r.get("revision")?,
    })
}

fn page_from_row(r: &Row<'_>) -> rusqlite::Result<PageRow> {
    Ok(PageRow {
        job_id: r.get("job_id")?,
        idx: r.get("idx")?,
        source_page: r.get("source_page")?,
        width: r.get("width")?,
        height: r.get("height")?,
        path: r.get("path")?,
        sha256: r.get("sha256")?,
        encoded_bytes: r.get("encoded_bytes")?,
    })
}

/// Write `data` to `path` via a flushed temporary file and atomic rename.
pub fn write_atomic(path: &Path, data: &[u8]) -> AppResult<()> {
    let tmp = path.with_extension("tmp");
    {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(data)?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

pub struct Store {
    conn: Connection,
    root: PathBuf,
    revision: i64,
    /// Held while the store is open: one NuPrint owns a data folder, its queue and its recovery.
    _lock: Option<fs::File>,
}

/// Take the exclusive owner lock of a data folder before anything reads or recovers its state. The
/// operating system releases it when the process ends, so a crash never leaves a folder locked.
fn lock_data_dir(data_dir: &Path) -> AppResult<fs::File> {
    let file = fs::OpenOptions::new().create(true).truncate(false).write(true).open(data_dir.join("state.lock"))?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(fs::TryLockError::WouldBlock) => {
            Err(AppError::new(ErrorCode::DataDirInUse, "Another NuPrint window is already using this data folder."))
        }
        Err(fs::TryLockError::Error(e)) => Err(e.into()),
    }
}

impl Store {
    /// Open (creating/migrating) the store in `data_dir`. Fails with [`ErrorCode::DataDirInUse`]
    /// while another process (or store) has it open.
    ///
    pub fn open(data_dir: &Path) -> AppResult<Self> {
        fs::create_dir_all(data_dir.join("jobs"))?;
        let db_path = data_dir.join("state.sqlite3");
        let lock = lock_data_dir(data_dir)?;
        let existed = db_path.exists();
        let conn = Connection::open(&db_path)?;
        Self::init(conn, data_dir.to_path_buf(), existed.then_some(db_path), Some(lock))
    }

    /// Non-persistent store (CLI one-shot use); assets go to `assets_dir`.
    pub fn open_in_memory(assets_dir: &Path) -> AppResult<Self> {
        fs::create_dir_all(assets_dir.join("jobs"))?;
        Self::init(Connection::open_in_memory()?, assets_dir.to_path_buf(), None, None)
    }

    fn init(conn: Connection, root: PathBuf, backup_from: Option<PathBuf>, lock: Option<fs::File>) -> AppResult<Self> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "journal_mode", "WAL").ok();
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version > SCHEMA_VERSION {
            return Err(AppError::new(
                ErrorCode::Internal,
                "This data was written by a newer version of NuPrint. Update NuPrint to open it.",
            )
            .with_detail(format!("schema {version} > supported {SCHEMA_VERSION}")));
        }
        if version < SCHEMA_VERSION {
            if version > 0
                && let Some(db) = &backup_from
            {
                // Back up before any transactional migration of an existing store.
                fs::copy(db, db.with_extension(format!("v{version}.bak")))?;
            }
            let migrations = [MIGRATION_1, MIGRATION_2, MIGRATION_3];
            conn.execute_batch("BEGIN")?;
            let result = (|| -> rusqlite::Result<()> {
                for (i, sql) in migrations.iter().enumerate().skip(version as usize) {
                    conn.execute_batch(sql)?;
                    conn.pragma_update(None, "user_version", (i + 1) as i64)?;
                }
                Ok(())
            })();
            match result {
                Ok(()) => conn.execute_batch("COMMIT")?,
                Err(e) => {
                    conn.execute_batch("ROLLBACK").ok();
                    return Err(e.into());
                }
            }
        }
        let revision = conn.query_row("SELECT COALESCE(MAX(revision), 0) FROM jobs", [], |r| r.get(0))?;
        let store = Self { conn, root, revision, _lock: lock };
        store.discard_temp_assets();
        Ok(store)
    }

    fn discard_temp_assets(&self) {
        let Ok(dirs) = fs::read_dir(self.root.join("jobs")) else { return };
        for dir in dirs.flatten() {
            let Ok(files) = fs::read_dir(dir.path()) else { continue };
            for file in files.flatten() {
                if file.path().extension().is_some_and(|e| e == "tmp") {
                    let _ = fs::remove_file(file.path());
                }
            }
        }
    }

    pub fn next_revision(&mut self) -> i64 {
        self.revision += 1;
        self.revision
    }

    pub fn revision(&self) -> i64 {
        self.revision
    }

    // ------------------------------------------------------------ preferences

    pub fn get_pref<T: DeserializeOwned>(&self, key: &str) -> AppResult<Option<T>> {
        let value: Option<String> =
            self.conn.query_row("SELECT value FROM preferences WHERE key = ?1", [key], |r| r.get(0)).optional()?;
        Ok(value.and_then(|v| serde_json::from_str(&v).ok()))
    }

    pub fn set_pref<T: Serialize>(&self, key: &str, value: &T) -> AppResult<()> {
        let json = serde_json::to_string(value).map_err(AppError::internal)?;
        self.conn.execute(
            "INSERT INTO preferences (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, json],
        )?;
        Ok(())
    }

    // ------------------------------------------------------------ printers

    pub fn list_printers(&self) -> AppResult<Vec<PrinterRow>> {
        let mut stmt = self.conn.prepare("SELECT * FROM printers ORDER BY created_ms")?;
        let rows = stmt.query_map([], printer_from_row)?.collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn get_printer(&self, id: &str) -> AppResult<Option<PrinterRow>> {
        Ok(self.conn.query_row("SELECT * FROM printers WHERE id = ?1", [id], printer_from_row).optional()?)
    }

    /// The saved printer for a device and built-in profile, including one whose settings were edited.
    pub fn find_printer_by_backend(&self, backend_id: &str, profile_id: &str) -> AppResult<Option<PrinterRow>> {
        Ok(self
            .conn
            .query_row(
                "SELECT * FROM printers WHERE backend_id = ?1 AND (profile_id = ?2 OR base_profile_id = ?2)",
                [backend_id, profile_id],
                printer_from_row,
            )
            .optional()?)
    }

    pub fn upsert_printer(&self, p: &PrinterRow) -> AppResult<()> {
        self.conn.execute(
            "INSERT INTO printers (id, name, backend_kind, backend_id, address, profile_id, profile_revision,
                                   base_profile_id, firmware, last_seen_ms, density, feed_mm, created_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name, backend_id = excluded.backend_id,
                 address = excluded.address, profile_id = excluded.profile_id,
                 profile_revision = excluded.profile_revision, base_profile_id = excluded.base_profile_id,
                 firmware = excluded.firmware,
                 last_seen_ms = excluded.last_seen_ms, density = excluded.density, feed_mm = excluded.feed_mm",
            params![
                p.id,
                p.name,
                p.backend_kind,
                p.backend_id,
                p.address,
                p.profile_id,
                p.profile_revision,
                p.base_profile_id,
                p.firmware,
                p.last_seen_ms,
                p.density.as_str(),
                p.feed_mm,
                p.created_ms
            ],
        )?;
        Ok(())
    }

    // ------------------------------------------------------------ custom profiles

    pub fn insert_custom_profile(&self, row: &CustomProfileRow, base_profile_id: &str) -> AppResult<()> {
        self.conn.execute(
            "INSERT INTO custom_profiles (id, revision, base_profile_id, json, created_ms) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![row.id, row.revision, base_profile_id, row.json, now_ms()],
        )?;
        Ok(())
    }

    /// Every stored revision, oldest first, so the registry can resolve queued jobs exactly.
    pub fn list_custom_profiles(&self) -> AppResult<Vec<CustomProfileRow>> {
        let mut stmt =
            self.conn.prepare("SELECT id, revision, json FROM custom_profiles ORDER BY created_ms, revision")?;
        let rows = stmt
            .query_map([], |r| Ok(CustomProfileRow { id: r.get(0)?, revision: r.get(1)?, json: r.get(2)? }))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn delete_printer(&self, id: &str) -> AppResult<bool> {
        Ok(self.conn.execute("DELETE FROM printers WHERE id = ?1", [id])? > 0)
    }

    // ------------------------------------------------------------ jobs

    fn job_dir(&self, job_id: &str) -> PathBuf {
        self.root.join("jobs").join(job_id)
    }

    /// Persist page assets, then commit job + page rows in one transaction.
    pub fn insert_job(&mut self, job: &JobRow, pages: &[(u32, MonoBitmap)], encoded_bytes: &[i64]) -> AppResult<()> {
        let dir = self.job_dir(&job.id);
        fs::create_dir_all(&dir)?;
        let mut rows = Vec::with_capacity(pages.len());
        for (idx, ((source_page, bitmap), encoded)) in pages.iter().zip(encoded_bytes).enumerate() {
            let name = format!("page-{idx}.bin");
            write_atomic(&dir.join(&name), bitmap.data())?;
            rows.push(PageRow {
                job_id: job.id.clone(),
                idx: idx as u32,
                source_page: *source_page,
                width: bitmap.width(),
                height: bitmap.height(),
                path: format!("jobs/{}/{name}", job.id),
                sha256: bitmap.sha256_hex(),
                encoded_bytes: *encoded,
            });
        }
        let tx = self.conn.transaction()?;
        insert_job_row(&tx, job)?;
        for p in &rows {
            tx.execute(
                "INSERT INTO pages (job_id, idx, source_page, width, height, path, sha256, encoded_bytes)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![p.job_id, p.idx, p.source_page, p.width, p.height, p.path, p.sha256, p.encoded_bytes],
            )?;
        }
        tx.commit().inspect_err(|_| {
            let _ = fs::remove_dir_all(&dir);
        })?;
        Ok(())
    }

    pub fn find_job_by_key(&self, key: &str) -> AppResult<Option<JobRow>> {
        Ok(self.conn.query_row("SELECT * FROM jobs WHERE idempotency_key = ?1", [key], job_from_row).optional()?)
    }

    pub fn get_job(&self, id: &str) -> AppResult<Option<JobRow>> {
        Ok(self.conn.query_row("SELECT * FROM jobs WHERE id = ?1", [id], job_from_row).optional()?)
    }

    pub fn list_jobs(&self, limit: usize) -> AppResult<Vec<JobRow>> {
        let mut stmt = self.conn.prepare("SELECT * FROM jobs ORDER BY created_ms DESC, rowid DESC LIMIT ?1")?;
        let rows = stmt.query_map([limit as i64], job_from_row)?.collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn next_queued(&self) -> AppResult<Option<JobRow>> {
        Ok(self
            .conn
            .query_row("SELECT * FROM jobs WHERE state = 'queued' ORDER BY created_ms, rowid LIMIT 1", [], job_from_row)
            .optional()?)
    }

    /// Claim a queued job for printing: the checked `queued` -> `connecting` transition and the
    /// attempt record, in one transaction. `None` means the job is no longer queued (cancelled or
    /// held in the meantime), so nothing may be sent. An error means the claim was not saved and
    /// nothing may be sent either; a claim that is saved survives a restart as an uncertain job.
    pub fn claim_queued(&mut self, job_id: &str, attempt_id: &str) -> AppResult<Option<JobRow>> {
        let revision = self.next_revision();
        let now = now_ms();
        let tx = self.conn.transaction()?;
        let changed = tx.execute(
            "UPDATE jobs SET state = 'connecting', updated_ms = ?2, revision = ?3 WHERE id = ?1 AND state = 'queued'",
            params![job_id, now, revision],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        tx.execute(
            "INSERT INTO attempts (id, job_id, started_ms, phase, page_cursor, copy_cursor, bytes_submitted)
             VALUES (?1, ?2, ?3, 'connecting', 0, 0, 0)",
            params![attempt_id, job_id, now],
        )?;
        tx.commit()?;
        self.get_job(job_id)
    }

    /// Cancel a job that has not started printing: a checked transition, so it cannot overwrite a
    /// job that was claimed first. `None` means the job was not queued or held.
    pub fn cancel_unstarted(&mut self, job_id: &str) -> AppResult<Option<JobRow>> {
        let revision = self.next_revision();
        let changed = self.conn.execute(
            "UPDATE jobs SET state = 'cancelled', updated_ms = ?2, revision = ?3
             WHERE id = ?1 AND state IN ('queued', 'held')",
            params![job_id, now_ms(), revision],
        )?;
        if changed == 0 { Ok(None) } else { self.get_job(job_id) }
    }

    pub fn pages(&self, job_id: &str) -> AppResult<Vec<PageRow>> {
        let mut stmt = self.conn.prepare("SELECT * FROM pages WHERE job_id = ?1 ORDER BY idx")?;
        let rows = stmt.query_map([job_id], page_from_row)?.collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// Every page of a job, verified, as (1-based source page, bitmap).
    pub fn load_pages(&self, job_id: &str) -> AppResult<Vec<(u32, MonoBitmap)>> {
        self.pages(job_id)?.iter().map(|page| Ok((page.source_page, self.load_page(page)?))).collect()
    }

    /// Load and verify a page's canonical bitmap against its recorded hash.
    pub fn load_page(&self, page: &PageRow) -> AppResult<MonoBitmap> {
        let corrupt = || {
            AppError::new(
                ErrorCode::RenderInvalid,
                "A saved print page is missing or damaged. Print it again from the source.",
            )
        };
        let path = self.root.join(&page.path);
        if !path.starts_with(&self.root) {
            return Err(corrupt());
        }
        let data = fs::read(&path).map_err(|e| corrupt().with_detail(e.to_string()))?;
        let bitmap =
            MonoBitmap::from_packed(page.width, page.height, data).map_err(|e| corrupt().with_detail(e.to_string()))?;
        if bitmap.sha256_hex() != page.sha256 {
            return Err(corrupt().with_detail("sha256 mismatch"));
        }
        Ok(bitmap)
    }

    pub fn update_job(&mut self, job: &mut JobRow) -> AppResult<()> {
        job.revision = self.next_revision();
        job.updated_ms = now_ms();
        let error_json = error_json(job.error.as_ref());
        self.conn.execute(
            "UPDATE jobs SET state = ?2, updated_ms = ?3, bytes_sent = ?4, error_json = ?5, partial_output = ?6,
                             content_retained = ?7, revision = ?8 WHERE id = ?1",
            params![
                job.id,
                job.state.as_str(),
                job.updated_ms,
                job.bytes_sent,
                error_json,
                job.partial_output,
                job.content_retained,
                job.revision
            ],
        )?;
        Ok(())
    }

    /// Delete a job's raster content but keep its metadata row.
    /// Delete a job's print data. `thumbnail` (a PNG of its first page) is the one thing kept, so
    /// the queue can still show what was printed; it goes with the job when the job is deleted.
    pub fn drop_content(&mut self, job: &mut JobRow, thumbnail: Option<&[u8]>) -> AppResult<()> {
        let dir = self.job_dir(&job.id);
        if dir.exists() {
            fs::remove_dir_all(&dir)?;
        }
        if let Some(png) = thumbnail {
            fs::create_dir_all(&dir)?;
            fs::write(dir.join(THUMBNAIL_FILE), png)?;
        }
        if job.content_retained {
            job.content_retained = false;
            self.update_job(job)?;
        }
        Ok(())
    }

    /// The kept preview of a job whose print data was deleted. `job_id` must come from a job row.
    pub fn thumbnail(&self, job_id: &str) -> Option<Vec<u8>> {
        fs::read(self.job_dir(job_id).join(THUMBNAIL_FILE)).ok()
    }

    pub fn delete_job(&mut self, id: &str) -> AppResult<()> {
        let dir = self.job_dir(id);
        if dir.exists() {
            fs::remove_dir_all(&dir)?;
        }
        self.conn.execute("DELETE FROM jobs WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn retained_content_bytes(&self) -> AppResult<i64> {
        Ok(self.conn.query_row(
            "SELECT COALESCE(SUM((p.width + 7) / 8 * p.height), 0) FROM pages p JOIN jobs j ON j.id = p.job_id
             WHERE j.content_retained = 1",
            [],
            |r| r.get(0),
        )?)
    }

    pub fn count_jobs_with_content(&self) -> AppResult<i64> {
        Ok(self.conn.query_row("SELECT COUNT(*) FROM jobs WHERE content_retained = 1", [], |r| r.get(0))?)
    }

    // ------------------------------------------------------------ attempts & events

    pub fn update_attempt(
        &self,
        attempt_id: &str,
        phase: JobState,
        page: u32,
        copy: u32,
        bytes: i64,
        last_status: Option<&str>,
    ) -> AppResult<()> {
        self.conn.execute(
            "UPDATE attempts SET phase = ?2, page_cursor = ?3, copy_cursor = ?4, bytes_submitted = ?5,
                                 last_status = COALESCE(?6, last_status) WHERE id = ?1",
            params![attempt_id, phase.as_str(), page, copy, bytes, last_status],
        )?;
        Ok(())
    }

    pub fn finish_attempt(&self, attempt_id: &str, outcome: JobState, error_code: Option<ErrorCode>) -> AppResult<()> {
        let code = error_code.map(|c| serde_json::to_value(c).ok().and_then(|v| v.as_str().map(str::to_owned)));
        self.conn.execute(
            "UPDATE attempts SET ended_ms = ?2, outcome = ?3, error_code = ?4 WHERE id = ?1",
            params![attempt_id, now_ms(), outcome.as_str(), code.flatten()],
        )?;
        Ok(())
    }

    /// Content-free operational event.
    pub fn add_event(&self, attempt_id: &str, kind: &str, details: &str) -> AppResult<()> {
        self.conn.execute(
            "INSERT INTO events (attempt_id, at_ms, kind, details) VALUES (?1, ?2, ?3, ?4)",
            params![attempt_id, now_ms(), kind, details],
        )?;
        Ok(())
    }

    // ------------------------------------------------------------ lifecycle

    /// Conservative restart recovery: active attempts become Uncertain (never replayed);
    /// queued jobs become Held until the user resumes. Returns (uncertain, held).
    pub fn recover_after_restart(&mut self) -> AppResult<(usize, usize)> {
        let jobs = self.list_jobs(10_000)?;
        let (mut uncertain, mut held) = (0, 0);
        for mut job in jobs {
            if job.state.is_active() {
                job.state = JobState::Uncertain;
                job.partial_output = true;
                job.error = Some(AppError::new(
                    ErrorCode::OutcomeUncertain,
                    "NuPrint closed while this job was printing. Check the paper; it was not resent.",
                ));
                self.update_job(&mut job)?;
                self.conn.execute(
                    "UPDATE attempts SET ended_ms = ?2, outcome = 'uncertain' WHERE job_id = ?1 AND ended_ms IS NULL",
                    params![job.id, now_ms()],
                )?;
                uncertain += 1;
            } else if job.state == JobState::Queued {
                job.state = JobState::Held;
                self.update_job(&mut job)?;
                held += 1;
            }
        }
        Ok((uncertain, held))
    }

    /// Apply retention: completed content is deleted on completion (by the job service);
    /// failed/uncertain/cancelled content is kept for recovery up to 7 days; terminal
    /// metadata is kept for 30 days.
    pub fn apply_retention(&mut self, now: i64) -> AppResult<()> {
        for mut job in self.list_jobs(100_000)? {
            if !job.state.is_terminal() {
                continue;
            }
            if job.updated_ms < now - COMPLETED_METADATA_DAYS * DAY_MS {
                self.delete_job(&job.id)?;
            } else if job.content_retained && job.updated_ms < now - FAILED_CONTENT_DAYS * DAY_MS {
                self.drop_content(&mut job, None)?;
            }
        }
        Ok(())
    }
}

fn insert_job_row(conn: &Connection, j: &JobRow) -> rusqlite::Result<()> {
    let error_json = error_json(j.error.as_ref());
    conn.execute(
        "INSERT INTO jobs (id, idempotency_key, title, source_kind, printer_id, printer_name, backend_id, profile_id,
             profile_revision, protocol_id, protocol_version, state, created_ms, updated_ms, copies, density, feed_mm,
             dots_per_mm, renderer_version, settings_json, content_retained, bytes_total, bytes_sent, error_json,
             partial_output, retry_of, revision)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22,
                 ?23, ?24, ?25, ?26, ?27)",
        params![
            j.id,
            j.idempotency_key,
            j.title,
            j.source_kind,
            j.printer_id,
            j.printer_name,
            j.backend_id,
            j.profile_id,
            j.profile_revision,
            j.protocol_id,
            j.protocol_version,
            j.state.as_str(),
            j.created_ms,
            j.updated_ms,
            j.copies,
            j.density.as_str(),
            j.feed_mm,
            j.dots_per_mm,
            j.renderer_version,
            j.settings_json,
            j.content_retained,
            j.bytes_total,
            j.bytes_sent,
            error_json,
            j.partial_output,
            j.retry_of,
            j.revision
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn job(id: &str, state: JobState) -> JobRow {
        JobRow {
            id: id.into(),
            idempotency_key: format!("key-{id}"),
            title: "Note".into(),
            source_kind: "text".into(),
            printer_id: "p1".into(),
            printer_name: "SC03h-TEST".into(),
            backend_id: "AA:BB".into(),
            profile_id: "sc03h-iprint-384".into(),
            profile_revision: 1,
            protocol_id: "iprint-5178".into(),
            protocol_version: 1,
            state,
            created_ms: now_ms(),
            updated_ms: now_ms(),
            copies: 1,
            density: Density::Normal,
            feed_mm: 12,
            dots_per_mm: 8.0,
            renderer_version: "test".into(),
            settings_json: "{}".into(),
            content_retained: true,
            bytes_total: 100,
            bytes_sent: 0,
            error: None,
            partial_output: false,
            retry_of: None,
            revision: 0,
        }
    }

    #[test]
    fn job_round_trip_with_verified_assets() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path()).unwrap();
        let bitmap = MonoBitmap::from_fn(384, 3, |x, y| x == y).unwrap();
        store.insert_job(&job("j1", JobState::Queued), &[(1, bitmap.clone())], &[205]).unwrap();
        let loaded = store.get_job("j1").unwrap().unwrap();
        assert_eq!(loaded.state, JobState::Queued);
        let pages = store.pages("j1").unwrap();
        assert_eq!(store.load_page(&pages[0]).unwrap(), bitmap);
        // Tampering is detected rather than printed.
        fs::write(dir.path().join(&pages[0].path), vec![0xFF; 48 * 3]).unwrap();
        assert_eq!(store.load_page(&pages[0]).unwrap_err().code, ErrorCode::RenderInvalid);
        // Duplicate idempotency key is rejected by the schema.
        let mut dup = job("j2", JobState::Queued);
        dup.idempotency_key = "key-j1".into();
        assert!(store.insert_job(&dup, &[(1, bitmap)], &[1]).is_err());
    }

    #[test]
    fn restart_recovery_never_replays() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut store = Store::open(dir.path()).unwrap();
            let b = MonoBitmap::new(384, 1).unwrap();
            for (id, state) in [("a", JobState::Sending), ("b", JobState::Queued), ("c", JobState::Completed)] {
                store.insert_job(&job(id, state), &[(1, b.clone())], &[1]).unwrap();
            }
        }
        let mut store = Store::open(dir.path()).unwrap();
        assert_eq!(store.recover_after_restart().unwrap(), (1, 1));
        let a = store.get_job("a").unwrap().unwrap();
        assert_eq!(a.state, JobState::Uncertain);
        assert!(a.partial_output);
        assert_eq!(store.get_job("b").unwrap().unwrap().state, JobState::Held);
        assert_eq!(store.get_job("c").unwrap().unwrap().state, JobState::Completed);
        assert!(store.next_queued().unwrap().is_none());
    }

    #[test]
    fn retention_and_preferences() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path()).unwrap();
        let b = MonoBitmap::new(384, 2).unwrap();
        let mut old_failed = job("old", JobState::Failed);
        old_failed.updated_ms = now_ms() - 8 * DAY_MS;
        store.insert_job(&old_failed, &[(1, b.clone())], &[1]).unwrap();
        let mut ancient = job("ancient", JobState::Completed);
        ancient.updated_ms = now_ms() - 31 * DAY_MS;
        store.insert_job(&ancient, &[(1, b)], &[1]).unwrap();
        assert_eq!(store.retained_content_bytes().unwrap(), 2 * 96);
        store.apply_retention(now_ms()).unwrap();
        assert!(store.get_job("ancient").unwrap().is_none());
        let old = store.get_job("old").unwrap().unwrap();
        assert!(!old.content_retained);
        assert!(!dir.path().join("jobs/old").exists());
        store.set_pref("queue_held", &true).unwrap();
        assert_eq!(store.get_pref::<bool>("queue_held").unwrap(), Some(true));
    }

    #[test]
    fn upgrading_from_schema_1_keeps_printers_and_adds_custom_profiles() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("state.sqlite3");
        {
            let conn = Connection::open(&db).unwrap();
            conn.execute_batch(MIGRATION_1).unwrap();
            conn.execute(
                "INSERT INTO printers (id, name, backend_kind, backend_id, profile_id, profile_revision, density,
                                       feed_mm, created_ms)
                 VALUES ('p1', 'SC03h-TEST', 'ble_gatt', 'dev', 'sc03h-iprint-384', 1, 'normal', 12, 1)",
                [],
            )
            .unwrap();
            conn.pragma_update(None, "user_version", 1).unwrap();
        }
        let store = Store::open(dir.path()).unwrap();
        assert!(dir.path().join("state.v1.bak").exists(), "the old store is backed up first");
        let mut printer = store.get_printer("p1").unwrap().unwrap();
        assert_eq!(printer.base_profile_id, None);
        assert_eq!(store.find_printer_by_backend("dev", "sc03h-iprint-384").unwrap().unwrap().id, "p1");

        // A printer switched to a custom profile is still found by its built-in profile.
        let row = CustomProfileRow { id: "custom:p1".into(), revision: 2, json: "{}".into() };
        store.insert_custom_profile(&row, "sc03h-iprint-384").unwrap();
        printer.profile_id = "custom:p1".into();
        printer.profile_revision = 2;
        printer.base_profile_id = Some("sc03h-iprint-384".into());
        store.upsert_printer(&printer).unwrap();
        assert_eq!(store.find_printer_by_backend("dev", "sc03h-iprint-384").unwrap().unwrap().profile_id, "custom:p1");
        assert_eq!(store.list_custom_profiles().unwrap(), vec![row]);
    }

    #[test]
    fn upgrading_from_schema_2_removes_saved_text_and_passwords() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut store = Store::open(dir.path()).unwrap();
            let b = MonoBitmap::new(384, 1).unwrap();
            for (id, settings) in [
                ("j1", r#"{"kind":"text","text":"my secret note","size_px":24}"#),
                ("j2", r#"{"kind":"pdf","source_id":"s","page":1,"password":"hunter2"}"#),
            ] {
                let mut row = job(id, JobState::Completed);
                row.settings_json = settings.into();
                store.insert_job(&row, &[(1, b.clone())], &[1]).unwrap();
            }
            store.conn.pragma_update(None, "user_version", 2).unwrap();
        }
        let store = Store::open(dir.path()).unwrap();
        assert!(dir.path().join("state.v2.bak").exists());
        let text = store.get_job("j1").unwrap().unwrap().settings_json;
        let pdf = store.get_job("j2").unwrap().unwrap().settings_json;
        assert!(!text.contains("secret") && text.contains("size_px"), "{text}");
        assert!(!pdf.contains("hunter2") && pdf.contains("source_id"), "{pdf}");
    }

    #[test]
    fn a_data_folder_has_one_owner_at_a_time() {
        let dir = tempfile::tempdir().unwrap();
        let mut first = Store::open(dir.path()).unwrap();
        let b = MonoBitmap::new(384, 1).unwrap();
        first.insert_job(&job("live", JobState::Sending), &[(1, b)], &[1]).unwrap();
        let refused = Store::open(dir.path()).err().expect("second owner is refused");
        assert_eq!(refused.code, ErrorCode::DataDirInUse);
        // The refusal did not recover (relabel) the live job.
        assert_eq!(first.get_job("live").unwrap().unwrap().state, JobState::Sending);
        drop(first);
        assert!(Store::open(dir.path()).is_ok(), "the folder is free once the owner closes");
    }

    #[test]
    fn a_job_is_claimed_or_cancelled_never_both() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path()).unwrap();
        let b = MonoBitmap::new(384, 1).unwrap();
        for id in ["claimed-first", "cancelled-first"] {
            store.insert_job(&job(id, JobState::Queued), &[(1, b.clone())], &[1]).unwrap();
        }
        // Claim wins: the cancel changes nothing and the attempt exists.
        let claimed = store.claim_queued("claimed-first", "a1").unwrap().expect("claimed");
        assert_eq!(claimed.state, JobState::Connecting);
        assert!(store.cancel_unstarted("claimed-first").unwrap().is_none());
        assert_eq!(store.get_job("claimed-first").unwrap().unwrap().state, JobState::Connecting);
        assert!(store.claim_queued("claimed-first", "a2").unwrap().is_none(), "no second claim");
        // Cancel wins: the stale queued row cannot be claimed.
        assert_eq!(store.cancel_unstarted("cancelled-first").unwrap().unwrap().state, JobState::Cancelled);
        assert!(store.claim_queued("cancelled-first", "a3").unwrap().is_none());
        assert_eq!(store.get_job("cancelled-first").unwrap().unwrap().state, JobState::Cancelled);
        let attempts: i64 = store.conn.query_row("SELECT COUNT(*) FROM attempts", [], |r| r.get(0)).unwrap();
        assert_eq!(attempts, 1, "only the winning claim recorded an attempt");
    }

    #[test]
    fn refuses_newer_schema() {
        let dir = tempfile::tempdir().unwrap();
        drop(Store::open(dir.path()).unwrap());
        let conn = Connection::open(dir.path().join("state.sqlite3")).unwrap();
        conn.pragma_update(None, "user_version", 99).unwrap();
        drop(conn);
        assert!(Store::open(dir.path()).is_err());
    }
}
