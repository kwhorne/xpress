//! Syncing the clipboard history between Macs through a shared folder —
//! by default `xpress/` in iCloud Drive.
//!
//! Each Mac writes what changes in its history as one JSON line per change to
//! its own log, and reads the other Macs' logs:
//!
//! ```text
//! xpress/
//!   devices/<device>/device.json      the Mac's name, when it last synced
//!   devices/<device>/log-000001.jsonl changes, oldest first
//!   blobs/<hash>.<ext>                images, written once
//! ```
//!
//! Only the owner writes to its folder, so there are no write conflicts;
//! iCloud only has to copy files. Changes merge by content (a clip's identity
//! is its content hash, so the same text copied on two Macs is one clip), the
//! newest use wins, and deletions travel as tombstones so an older copy
//! elsewhere doesn't bring a clip back. A Mac that turns syncing on, or whose
//! logs grow large, writes a fresh snapshot of everything and drops its old
//! logs.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::history::{now_ms, ClipKind, History, Retention, Rule};

/// One change, as written to a log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum Op {
    Add {
        hash: String,
        kind: String,
        text: String,
        #[serde(default)]
        ocr: String,
        /// The image's file extension; the image is `blobs/<hash>.<ext>`.
        #[serde(default)]
        image: Option<String>,
        #[serde(default)]
        app: Option<String>,
        #[serde(default)]
        bundle: Option<String>,
        created: i64,
        used: i64,
        #[serde(default)]
        pinned: bool,
    },
    Touch {
        hash: String,
        used: i64,
    },
    Pin {
        hash: String,
        pinned: bool,
    },
    Ocr {
        hash: String,
        text: String,
    },
    Delete {
        hash: String,
        at: i64,
    },
    Multi {
        hash: String,
        items: Vec<String>,
        created: i64,
        used: i64,
        #[serde(default)]
        pinned: bool,
    },
    Category {
        name: String,
        /// The old name, when it was renamed.
        #[serde(default)]
        previous: Option<String>,
        color: String,
        #[serde(default)]
        app: Option<String>,
        #[serde(default)]
        kind: Option<String>,
        #[serde(default)]
        contains: Option<String>,
    },
    CategoryDelete {
        name: String,
    },
    Categorize {
        hash: String,
        category: String,
        on: bool,
    },
}

/// Logs are started afresh after this many lines.
const LOG_LINES: usize = 1000;
/// Write a snapshot instead when a Mac's logs grow past this.
const COMPACT_FILES: usize = 20;
const COMPACT_BYTES: u64 = 16_000_000;
/// An image that hasn't arrived after this long is given up on.
const BLOB_PATIENCE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// Tombstones are forgotten after this long.
const TOMBSTONE_DAYS: i64 = 180;

/// What a sync round did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// Changes written for the other Macs.
    pub sent: usize,
    /// Changes taken over from them.
    pub received: usize,
    /// Changes waiting for an image that hasn't arrived yet.
    pub waiting: usize,
    /// The other Macs syncing here.
    pub devices: Vec<String>,
}

/// `~/Library/Mobile Documents/com~apple~CloudDocs`, when iCloud Drive is on.
pub fn icloud_drive() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let drive = home.join("Library/Mobile Documents/com~apple~CloudDocs");
    drive.is_dir().then_some(drive)
}

/// The folder in iCloud Drive that xpress syncs through.
pub fn default_root() -> Option<PathBuf> {
    icloud_drive().map(|d| d.join("xpress"))
}

/// This Mac's name, as shown to the others.
pub fn device_name() -> String {
    let scutil = std::process::Command::new("scutil")
        .args(["--get", "ComputerName"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty());
    scutil
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_else(|| "Mac".into())
}

#[derive(Serialize, Deserialize)]
struct DeviceInfo {
    name: String,
    last_sync: i64,
}

/// One sync round: share this Mac's changes and take over the others'.
/// `retention` keeps old clips from other Macs from coming in only to be
/// pruned again.
pub fn sync(
    history: &Mutex<History>,
    root: &Path,
    name: &str,
    retention: Retention,
) -> std::io::Result<Report> {
    sync_with(history, root, name, retention, false)
}

/// [`sync`], starting from a fresh snapshot of this Mac's history with
/// `snapshot` — after syncing was off, changes weren't recorded.
pub fn sync_with(
    history: &Mutex<History>,
    root: &Path,
    name: &str,
    retention: Retention,
    snapshot: bool,
) -> std::io::Result<Report> {
    let device = history.lock().unwrap().device_id().map_err(io)?;
    let own = root.join("devices").join(&device);
    fs::create_dir_all(&own)?;
    fs::create_dir_all(root.join("blobs"))?;
    write_readme(root);

    let mut report = Report::default();
    if snapshot || log_files(&own)?.is_empty() || needs_compaction(&own)? {
        compact(history, root, &own)?;
    }
    report.sent = flush(history, root, &own)?;

    for entry in fs::read_dir(root.join("devices"))?.flatten() {
        let dir = entry.path();
        let id = entry.file_name().to_string_lossy().into_owned();
        if id == device || !dir.is_dir() {
            continue;
        }
        if let Ok(info) = fs::read(dir.join("device.json")) {
            if let Ok(info) = serde_json::from_slice::<DeviceInfo>(&info) {
                report.devices.push(info.name);
            }
        }
        let (received, waiting) = pull(history, root, &id, &dir, retention)?;
        report.received += received;
        report.waiting += waiting;
    }

    let info = DeviceInfo {
        name: name.to_string(),
        last_sync: now_ms(),
    };
    write_atomic(
        &own.join("device.json"),
        &serde_json::to_vec_pretty(&info).map_err(io)?,
    )?;
    history
        .lock()
        .unwrap()
        .forget_old_tombstones()
        .map_err(io)?;
    Ok(report)
}

fn io<E: std::fmt::Display>(e: E) -> std::io::Error {
    std::io::Error::other(e.to_string())
}

fn write_readme(root: &Path) {
    let readme = root.join("README.txt");
    if !readme.exists() {
        let _ = fs::write(
            &readme,
            "This folder keeps the xpress clipboard history in sync between your Macs.\n\
             Turn syncing off in xpress → Preferences before deleting it.\n",
        );
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(tmp, path)
}

/// `log-000001.jsonl`, … sorted.
fn log_files(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut files: Vec<PathBuf> = match fs::read_dir(dir) {
        Ok(entries) => entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("log-") && n.ends_with(".jsonl"))
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    files.sort();
    Ok(files)
}

fn log_number(path: &Path) -> u64 {
    path.file_stem()
        .and_then(|s| s.to_str())
        .and_then(|s| s.strip_prefix("log-"))
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

fn log_path(dir: &Path, n: u64) -> PathBuf {
    dir.join(format!("log-{n:06}.jsonl"))
}

fn needs_compaction(own: &Path) -> std::io::Result<bool> {
    let files = log_files(own)?;
    let bytes: u64 = files
        .iter()
        .filter_map(|f| f.metadata().ok())
        .map(|m| m.len())
        .sum();
    Ok(files.len() > COMPACT_FILES || bytes > COMPACT_BYTES)
}

/// Append lines to this Mac's newest log (a new one when it's full).
fn append(own: &Path, lines: &[String]) -> std::io::Result<()> {
    let mut lines = lines.iter().peekable();
    while lines.peek().is_some() {
        let files = log_files(own)?;
        let (path, used) = match files.last() {
            Some(last) => {
                let used = fs::read_to_string(last)?.lines().count();
                if used >= LOG_LINES {
                    (log_path(own, log_number(last) + 1), 0)
                } else {
                    (last.clone(), used)
                }
            }
            None => (log_path(own, 1), 0),
        };
        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
        let mut chunk = String::new();
        for _ in used..LOG_LINES {
            match lines.next() {
                Some(line) => {
                    chunk.push_str(line);
                    chunk.push('\n');
                }
                None => break,
            }
        }
        file.write_all(chunk.as_bytes())?;
        file.sync_all()?;
    }
    Ok(())
}

/// An image to share: its blob name and where it is here.
type Blob = (String, PathBuf);

/// The images `ops` need, looked up while the history is locked.
fn blobs_for<'a>(
    history: &History,
    ops: impl Iterator<Item = &'a Op>,
) -> std::io::Result<Vec<Blob>> {
    let mut blobs = Vec::new();
    for op in ops {
        if let Op::Add {
            hash,
            image: Some(ext),
            ..
        } = op
        {
            if let Some(src) = history.image_path(hash).map_err(io)? {
                blobs.push((format!("{hash}.{ext}"), src));
            }
        }
    }
    Ok(blobs)
}

/// Copy images to the shared blobs, once each — without holding the history
/// lock, since a first sync can copy a lot.
fn share_blobs(root: &Path, blobs: &[Blob]) -> std::io::Result<()> {
    for (name, src) in blobs {
        let blob = root.join("blobs").join(name);
        if blob.exists() || !src.exists() {
            continue;
        }
        let tmp = blob.with_extension("part");
        fs::copy(src, &tmp)?;
        fs::rename(tmp, blob)?;
    }
    Ok(())
}

/// Write the outbox to this Mac's log.
fn flush(history: &Mutex<History>, root: &Path, own: &Path) -> std::io::Result<usize> {
    let (pending, blobs) = {
        let h = history.lock().unwrap();
        let pending = h.outbox().map_err(io)?;
        let ops: Vec<Op> = pending
            .iter()
            .filter_map(|(_, json)| serde_json::from_str(json).ok())
            .collect();
        let blobs = blobs_for(&h, ops.iter())?;
        (pending, blobs)
    };
    if pending.is_empty() {
        return Ok(0);
    }
    share_blobs(root, &blobs)?;
    let lines: Vec<String> = pending.iter().map(|(_, json)| json.clone()).collect();
    append(own, &lines)?;
    // Only what was written; newer changes go next time.
    history
        .lock()
        .unwrap()
        .clear_outbox(pending.last().map(|(id, _)| *id).unwrap_or(0))
        .map_err(io)?;
    Ok(pending.len())
}

/// Replace this Mac's logs with one snapshot of everything.
fn compact(history: &Mutex<History>, root: &Path, own: &Path) -> std::io::Result<()> {
    let (ops, blobs) = {
        let h = history.lock().unwrap();
        let ops = h.snapshot().map_err(io)?;
        let blobs = blobs_for(&h, ops.iter())?;
        // The snapshot covers what's waiting too.
        h.clear_outbox(i64::MAX).map_err(io)?;
        (ops, blobs)
    };
    share_blobs(root, &blobs)?;
    let old = log_files(own)?;
    let next = old.last().map_or(1, |l| log_number(l) + 1);
    let lines: Vec<String> = ops
        .iter()
        .map(|op| serde_json::to_string(op).map_err(io))
        .collect::<std::io::Result<_>>()?;
    let first = log_path(own, next);
    append_from(own, &first, &lines)?;
    for file in old {
        let _ = fs::remove_file(file);
    }
    Ok(())
}

/// Like [`append`], but starting at `first` (a new, empty log).
fn append_from(own: &Path, first: &Path, lines: &[String]) -> std::io::Result<()> {
    fs::write(first, "")?;
    // Older logs still exist at this point; write past them.
    let mut chunks = lines.chunks(LOG_LINES);
    let mut n = log_number(first);
    if let Some(chunk) = chunks.next() {
        fs::write(
            first,
            chunk.join("\n") + if chunk.is_empty() { "" } else { "\n" },
        )?;
    }
    for chunk in chunks {
        n += 1;
        fs::write(log_path(own, n), chunk.join("\n") + "\n")?;
    }
    Ok(())
}

/// Take over another Mac's new changes. Returns (applied, waiting).
fn pull(
    history: &Mutex<History>,
    root: &Path,
    device: &str,
    dir: &Path,
    retention: Retention,
) -> std::io::Result<(usize, usize)> {
    let mut applied = 0;
    for file in log_files(dir)? {
        let name = file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let done = history
            .lock()
            .unwrap()
            .progress(device, &name)
            .map_err(io)?;
        // Not downloaded yet, or being written: try again next time.
        let Ok(content) = fs::read_to_string(&file) else {
            continue;
        };
        // Only whole lines; the last one may still be on its way.
        let complete = match content.rfind('\n') {
            Some(end) => &content[..=end],
            None => "",
        };
        let lines: Vec<&str> = complete.lines().collect();
        let mut line_no = done;
        for line in lines.iter().skip(done) {
            let Ok(op) = serde_json::from_str::<Op>(line) else {
                line_no += 1;
                continue;
            };
            let h = history.lock().unwrap();
            match h.apply(&op, root, retention).map_err(io)? {
                Applied::Done => applied += 1,
                Applied::Skipped => {}
                Applied::Wait => {
                    h.set_progress(device, &name, line_no).map_err(io)?;
                    return Ok((applied, lines.len() - line_no));
                }
            }
            line_no += 1;
            if line_no % 100 == 0 {
                h.set_progress(device, &name, line_no).map_err(io)?;
            }
        }
        history
            .lock()
            .unwrap()
            .set_progress(device, &name, line_no)
            .map_err(io)?;
    }
    Ok((applied, 0))
}

/// What became of a change from another Mac.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Applied {
    Done,
    /// Not needed here (deleted since, too old, already known).
    Skipped,
    /// Its image hasn't arrived yet.
    Wait,
}

impl History {
    /// This Mac's id in the shared folder (made once).
    pub fn device_id(&self) -> rusqlite::Result<String> {
        if let Some(id) = self
            .conn
            .query_row("SELECT value FROM meta WHERE key = 'device'", [], |r| {
                r.get(0)
            })
            .optional()?
        {
            return Ok(id);
        }
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(now_ms().to_le_bytes());
        h.update(std::process::id().to_le_bytes());
        h.update(self.dir.to_string_lossy().as_bytes());
        h.update(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
                .to_le_bytes(),
        );
        let id: String = h.finalize()[..8]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        self.conn
            .execute("INSERT INTO meta (key, value) VALUES ('device', ?1)", [&id])?;
        Ok(id)
    }

    fn outbox(&self) -> rusqlite::Result<Vec<(i64, String)>> {
        let mut stmt = self.conn.prepare("SELECT id, op FROM outbox ORDER BY id")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect()
    }

    fn clear_outbox(&self, up_to: i64) -> rusqlite::Result<()> {
        self.conn
            .execute("DELETE FROM outbox WHERE id <= ?1", [up_to])
            .map(|_| ())
    }

    fn progress(&self, device: &str, file: &str) -> rusqlite::Result<usize> {
        Ok(self
            .conn
            .query_row(
                "SELECT lines FROM sync_progress WHERE device = ?1 AND file = ?2",
                [device, file],
                |r| r.get::<_, i64>(0),
            )
            .optional()?
            .unwrap_or(0) as usize)
    }

    fn set_progress(&self, device: &str, file: &str, lines: usize) -> rusqlite::Result<()> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO sync_progress (device, file, lines) VALUES (?1, ?2, ?3)",
                params![device, file, lines as i64],
            )
            .map(|_| ())
    }

    fn image_path(&self, hash: &str) -> rusqlite::Result<Option<PathBuf>> {
        let rel: Option<Option<String>> = self
            .conn
            .query_row("SELECT image FROM clips WHERE hash = ?1", [hash], |r| {
                r.get(0)
            })
            .optional()?;
        Ok(rel.flatten().map(|r| self.dir.join(r)))
    }

    fn tombstone(&self, hash: &str) -> rusqlite::Result<Option<i64>> {
        self.conn
            .query_row("SELECT at FROM tombstones WHERE hash = ?1", [hash], |r| {
                r.get(0)
            })
            .optional()
    }

    fn forget_old_tombstones(&self) -> rusqlite::Result<()> {
        let cutoff = now_ms() - TOMBSTONE_DAYS * 24 * 60 * 60 * 1000;
        self.conn
            .execute("DELETE FROM tombstones WHERE at < ?1", [cutoff])
            .map(|_| ())
    }

    /// Everything, as changes: categories, clips, multi-clips, what's in
    /// which category, and recent deletions.
    fn snapshot(&self) -> rusqlite::Result<Vec<Op>> {
        let mut ops = Vec::new();
        for c in self.categories()? {
            ops.push(Op::Category {
                name: c.name,
                previous: None,
                color: format!("#{:02x}{:02x}{:02x}", c.color[0], c.color[1], c.color[2]),
                app: c.rule.app,
                kind: c.rule.kind.map(|k| k.as_str().to_string()),
                contains: c.rule.contains,
            });
        }
        let mut stmt = self.conn.prepare(
            "SELECT id, hash, kind, text, ocr, image, source_app, source_bundle, created,
                    last_used, pinned FROM clips ORDER BY last_used ASC, id ASC",
        )?;
        type Row = (
            i64,
            String,
            String,
            String,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            i64,
            i64,
            bool,
        );
        let rows: Vec<Row> = stmt
            .query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                    r.get(9)?,
                    r.get(10)?,
                ))
            })?
            .collect::<rusqlite::Result<_>>()?;
        let mut multis = Vec::new();
        for (id, hash, kind, text, ocr, image, app, bundle, created, used, pinned) in rows {
            if kind == ClipKind::Multi.as_str() {
                multis.push(id);
                continue;
            }
            let ext = image.as_deref().and_then(|p| {
                Path::new(p)
                    .extension()
                    .map(|e| e.to_string_lossy().into_owned())
            });
            ops.push(Op::Add {
                hash,
                kind,
                text,
                ocr,
                image: ext,
                app,
                bundle,
                created,
                used,
                pinned,
            });
        }
        for id in multis {
            let Some(clip) = self.get(id)? else { continue };
            let items = self
                .items(id)?
                .into_iter()
                .filter_map(|c| self.hash_of(c.id).ok().flatten())
                .collect();
            ops.push(Op::Multi {
                hash: self.hash_of(id)?.unwrap_or_default(),
                items,
                created: clip.created,
                used: clip.last_used,
                pinned: clip.pinned,
            });
        }
        let categories = self.categories()?;
        let mut stmt = self.conn.prepare(
            "SELECT clips.hash, clip_categories.category_id FROM clip_categories
             JOIN clips ON clips.id = clip_categories.clip_id",
        )?;
        let pairs: Vec<(String, i64)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (hash, category) in pairs {
            if let Some(c) = categories.iter().find(|c| c.id == category) {
                ops.push(Op::Categorize {
                    hash,
                    category: c.name.clone(),
                    on: true,
                });
            }
        }
        let mut stmt = self.conn.prepare("SELECT hash, at FROM tombstones")?;
        let deleted: Vec<(String, i64)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (hash, at) in deleted {
            ops.push(Op::Delete { hash, at });
        }
        Ok(ops)
    }

    /// Apply a change from another Mac (without sharing it again).
    pub(crate) fn apply(
        &self,
        op: &Op,
        root: &Path,
        retention: Retention,
    ) -> rusqlite::Result<Applied> {
        self.quietly(|| self.apply_inner(op, root, retention))
    }

    fn apply_inner(&self, op: &Op, root: &Path, retention: Retention) -> rusqlite::Result<Applied> {
        let now = now_ms();
        match op {
            Op::Add {
                hash,
                kind,
                text,
                ocr,
                image,
                app,
                bundle,
                created,
                used,
                pinned,
            } => {
                if self.tombstone(hash)?.is_some_and(|at| at >= *used) {
                    return Ok(Applied::Skipped);
                }
                if let Some(id) = self.id_of(hash)? {
                    self.conn.execute(
                        "UPDATE clips SET last_used = MAX(last_used, ?2),
                             pinned = CASE WHEN ?2 > last_used THEN ?3 ELSE pinned END,
                             ocr = CASE WHEN ocr = '' THEN ?4 ELSE ocr END
                         WHERE id = ?1",
                        params![id, used, pinned, ocr],
                    )?;
                    return Ok(Applied::Skipped);
                }
                let too_old = retention
                    .max_age
                    .is_some_and(|age| *used < now - age.as_millis() as i64);
                if too_old && !pinned {
                    return Ok(Applied::Skipped);
                }
                let Some(kind) = ClipKind::from_str(kind) else {
                    return Ok(Applied::Skipped);
                };
                let (image_rel, thumb, bytes) = match image {
                    // Names from another Mac's log must stay file names.
                    Some(ext) if !crate::history::safe_file_name(hash, ext) => {
                        return Ok(Applied::Skipped);
                    }
                    Some(ext) => {
                        let blob = root.join("blobs").join(format!("{hash}.{ext}"));
                        match fs::read(&blob) {
                            Ok(data) => self.store_image(hash, &data, ext),
                            Err(_) if now - used < BLOB_PATIENCE.as_millis() as i64 => {
                                return Ok(Applied::Wait);
                            }
                            Err(_) => return Ok(Applied::Skipped),
                        }
                    }
                    None => (None, None, text.len() as u64),
                };
                self.conn.execute(
                    "INSERT INTO clips (kind, hash, text, ocr, image, thumb, bytes, source_app,
                                        source_bundle, created, last_used, pinned)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                    params![
                        kind.as_str(),
                        hash,
                        text,
                        ocr,
                        image_rel,
                        thumb,
                        bytes as i64,
                        app,
                        bundle,
                        created,
                        used,
                        pinned
                    ],
                )?;
                let id = self.conn.last_insert_rowid();
                self.apply_rules(id)?;
                Ok(Applied::Done)
            }
            Op::Touch { hash, used } => match self.id_of(hash)? {
                Some(id) => {
                    self.conn.execute(
                        "UPDATE clips SET last_used = MAX(last_used, ?2) WHERE id = ?1",
                        params![id, used],
                    )?;
                    Ok(Applied::Done)
                }
                None => Ok(Applied::Skipped),
            },
            Op::Pin { hash, pinned } => match self.id_of(hash)? {
                Some(id) => {
                    self.set_pinned(id, *pinned)?;
                    Ok(Applied::Done)
                }
                None => Ok(Applied::Skipped),
            },
            Op::Ocr { hash, text } => match self.id_of(hash)? {
                Some(id) => {
                    self.set_ocr(id, text)?;
                    Ok(Applied::Done)
                }
                None => Ok(Applied::Skipped),
            },
            Op::Delete { hash, at } => {
                self.conn.execute(
                    "INSERT INTO tombstones (hash, at) VALUES (?1, ?2)
                     ON CONFLICT(hash) DO UPDATE SET at = MAX(at, excluded.at)",
                    params![hash, at],
                )?;
                match self.id_of(hash)? {
                    // Copied again here after it was deleted there: keep it.
                    Some(id) if self.get(id)?.is_some_and(|c| c.last_used <= *at) => {
                        self.delete(id)?;
                        Ok(Applied::Done)
                    }
                    _ => Ok(Applied::Skipped),
                }
            }
            Op::Multi {
                hash,
                items,
                created,
                used,
                pinned,
            } => {
                if self.tombstone(hash)?.is_some_and(|at| at >= *used) {
                    return Ok(Applied::Skipped);
                }
                let mut ids = Vec::new();
                for item in items {
                    match self.id_of(item)? {
                        Some(id) => ids.push(id),
                        // An item from a Mac not read yet: wait a little.
                        None if now - used < 24 * 60 * 60 * 1000
                            && self.tombstone(item)?.is_none() =>
                        {
                            return Ok(Applied::Wait);
                        }
                        None => {}
                    }
                }
                if ids.is_empty() {
                    return Ok(Applied::Skipped);
                }
                let multi = match self.id_of(hash)? {
                    Some(id) => {
                        self.conn
                            .execute("DELETE FROM clip_items WHERE parent_id = ?1", [id])?;
                        // The trigger removed the emptied multi-clip; make it again.
                        self.id_of(hash)?
                    }
                    None => None,
                };
                let multi = match multi {
                    Some(id) => id,
                    None => {
                        self.conn.execute(
                            "INSERT INTO clips (kind, hash, created, last_used, pinned)
                             VALUES ('multi', ?1, ?2, ?3, ?4)",
                            params![hash, created, used, pinned],
                        )?;
                        self.conn.last_insert_rowid()
                    }
                };
                for (pos, id) in ids.iter().enumerate() {
                    self.conn.execute(
                        "INSERT INTO clip_items (parent_id, position, child_id) VALUES (?1, ?2, ?3)",
                        params![multi, pos as i64, id],
                    )?;
                }
                self.conn.execute(
                    "UPDATE clips SET last_used = MAX(last_used, ?2), pinned = ?3 WHERE id = ?1",
                    params![multi, used, pinned],
                )?;
                self.refresh_multi(multi)?;
                Ok(Applied::Done)
            }
            Op::Category {
                name,
                previous,
                color,
                app,
                kind,
                contains,
            } => {
                let find = |n: &str| -> rusqlite::Result<Option<i64>> {
                    self.conn
                        .query_row(
                            "SELECT id FROM categories WHERE name = ?1 COLLATE NOCASE",
                            [n],
                            |r| r.get(0),
                        )
                        .optional()
                };
                let existing = match previous {
                    Some(old) => find(old)?.or(find(name)?),
                    None => find(name)?,
                };
                let rgb = crate::history::parse_color(color)
                    .map_or([122, 162, 247], |[r, g, b, _]| [r, g, b]);
                let rule = Rule {
                    app: app.clone(),
                    kind: kind.as_deref().and_then(ClipKind::from_str),
                    contains: contains.clone(),
                };
                self.save_category(existing, name, rgb, &rule)?;
                Ok(Applied::Done)
            }
            Op::CategoryDelete { name } => {
                let id: Option<i64> = self
                    .conn
                    .query_row(
                        "SELECT id FROM categories WHERE name = ?1 COLLATE NOCASE",
                        [name],
                        |r| r.get(0),
                    )
                    .optional()?;
                match id {
                    Some(id) => {
                        self.delete_category(id)?;
                        Ok(Applied::Done)
                    }
                    None => Ok(Applied::Skipped),
                }
            }
            Op::Categorize { hash, category, on } => {
                let category: Option<i64> = self
                    .conn
                    .query_row(
                        "SELECT id FROM categories WHERE name = ?1 COLLATE NOCASE",
                        [category],
                        |r| r.get(0),
                    )
                    .optional()?;
                match (self.id_of(hash)?, category) {
                    (Some(clip), Some(category)) => {
                        self.set_category(clip, category, *on)?;
                        Ok(Applied::Done)
                    }
                    _ => Ok(Applied::Skipped),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{NewClip, Query};

    struct Mac {
        _dir: tempfile::TempDir,
        history: Mutex<History>,
    }

    fn mac() -> Mac {
        let dir = tempfile::tempdir().unwrap();
        let history = History::open(dir.path()).unwrap();
        history.set_journal(true);
        Mac {
            _dir: dir,
            history: Mutex::new(history),
        }
    }

    const KEEP: Retention = Retention {
        max_age: None,
        max_bytes: None,
    };

    fn all() -> Query {
        Query {
            limit: 100_000,
            ..Default::default()
        }
    }

    impl Mac {
        fn sync(&self, cloud: &Path) -> Report {
            sync(&self.history, cloud, "Test Mac", KEEP).unwrap()
        }
        fn texts(&self) -> Vec<String> {
            self.history
                .lock()
                .unwrap()
                .search(&all())
                .unwrap()
                .into_iter()
                .map(|c| c.text)
                .collect()
        }
        fn h(&self) -> std::sync::MutexGuard<'_, History> {
            self.history.lock().unwrap()
        }
        fn id(&self, text: &str) -> i64 {
            self.h()
                .search(&all())
                .unwrap()
                .into_iter()
                .find(|c| c.text == text)
                .unwrap()
                .id
        }
    }

    fn png(shade: u8) -> Vec<u8> {
        let mut out = Vec::new();
        image::RgbImage::from_pixel(40, 30, image::Rgb([shade, 10, 10]))
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    #[test]
    fn clips_travel_between_macs() {
        let cloud = tempfile::tempdir().unwrap();
        let (a, b) = (mac(), mac());
        a.h()
            .add(NewClip::text("from A").from_app(Some("Mail".into()), None))
            .unwrap();
        let img = a.h().add(NewClip::image_png(png(1))).unwrap().id;
        a.h().set_ocr(img, "receipt 42").unwrap();

        assert_eq!(
            a.sync(cloud.path()).sent,
            0,
            "the first sync writes a snapshot"
        );
        let report = b.sync(cloud.path());
        assert_eq!(report.received, 2);
        assert_eq!(report.devices, ["Test Mac"]);

        let clips = b.h().search(&Query::default()).unwrap();
        let text = clips.iter().find(|c| c.text == "from A").unwrap();
        assert_eq!(text.source_app.as_deref(), Some("Mail"));
        let image = clips.iter().find(|c| c.kind == ClipKind::Image).unwrap();
        assert_eq!(image.ocr, "receipt 42");
        assert!(image.image.as_ref().unwrap().exists());
        assert!(image.thumb.as_ref().unwrap().exists());

        // Nothing echoes back.
        assert_eq!(b.sync(cloud.path()).sent, 0);
        assert_eq!(a.sync(cloud.path()).received, 0);
    }

    #[test]
    fn changes_pins_and_deletions_follow() {
        let cloud = tempfile::tempdir().unwrap();
        let (a, b) = (mac(), mac());
        a.sync(cloud.path());
        b.sync(cloud.path());
        a.h().add(NewClip::text("keep me")).unwrap();
        a.h().add(NewClip::text("drop me")).unwrap();
        a.sync(cloud.path());
        b.sync(cloud.path());
        assert_eq!(b.texts().len(), 2);

        let keep = b.id("keep me");
        b.h().set_pinned(keep, true).unwrap();
        let drop = b.id("drop me");
        b.h().delete(drop).unwrap();
        assert!(b.sync(cloud.path()).sent >= 2);
        a.sync(cloud.path());
        assert_eq!(a.texts(), ["keep me"]);
        let kept = a.id("keep me");
        assert!(a.h().get(kept).unwrap().unwrap().pinned);

        // An old copy doesn't bring it back…
        let c = mac();
        c.sync(cloud.path());
        assert_eq!(c.texts(), ["keep me"]);
        // …but copying it again does.
        a.h().add(NewClip::text("drop me")).unwrap();
        a.sync(cloud.path());
        b.sync(cloud.path());
        assert!(b.texts().contains(&"drop me".to_string()));
    }

    #[test]
    fn the_same_copy_on_two_macs_is_one_clip() {
        let cloud = tempfile::tempdir().unwrap();
        let (a, b) = (mac(), mac());
        a.h().add(NewClip::text("same")).unwrap();
        b.h().add(NewClip::text("same")).unwrap();
        a.sync(cloud.path());
        b.sync(cloud.path());
        a.sync(cloud.path());
        assert_eq!(a.texts(), ["same"]);
        assert_eq!(b.texts(), ["same"]);
    }

    #[test]
    fn categories_and_multi_clips_follow() {
        let cloud = tempfile::tempdir().unwrap();
        let (a, b) = (mac(), mac());
        a.sync(cloud.path());
        b.sync(cloud.path());
        let one = a.h().add(NewClip::text("one")).unwrap().id;
        let two = a.h().add(NewClip::text("two")).unwrap().id;
        let work = a
            .h()
            .save_category(None, "Work", [1, 2, 3], &Rule::default())
            .unwrap();
        a.h().set_category(one, work, true).unwrap();
        a.h().combine(&[two, one]).unwrap();
        a.sync(cloud.path());
        b.sync(cloud.path());

        let cats = b.h().categories().unwrap();
        assert_eq!(cats.len(), 1);
        assert_eq!(
            (cats[0].name.as_str(), cats[0].count, cats[0].color),
            ("Work", 1, [1, 2, 3])
        );
        let multi = b
            .h()
            .search(&Query {
                kind: Some(ClipKind::Multi),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(multi.len(), 1);
        let items: Vec<String> = b
            .h()
            .items(multi[0].id)
            .unwrap()
            .into_iter()
            .map(|c| c.text)
            .collect();
        assert_eq!(items, ["two", "one"]);

        // Rename on B, delete on A.
        let id = b.h().categories().unwrap()[0].id;
        b.h()
            .save_category(Some(id), "Job", [9, 9, 9], &Rule::default())
            .unwrap();
        b.sync(cloud.path());
        a.sync(cloud.path());
        assert_eq!(a.h().categories().unwrap()[0].name, "Job");
        let id = a.h().categories().unwrap()[0].id;
        a.h().delete_category(id).unwrap();
        a.sync(cloud.path());
        b.sync(cloud.path());
        assert!(b.h().categories().unwrap().is_empty());
    }

    #[test]
    fn images_wait_for_their_file() {
        let cloud = tempfile::tempdir().unwrap();
        let (a, b) = (mac(), mac());
        a.sync(cloud.path());
        a.h().add(NewClip::image_png(png(7))).unwrap();
        a.h().add(NewClip::text("after the image")).unwrap();
        a.sync(cloud.path());
        // The image hasn't reached this Mac yet.
        let blob = fs::read_dir(cloud.path().join("blobs"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let hidden = blob.with_extension("hidden");
        fs::rename(&blob, &hidden).unwrap();
        let report = b.sync(cloud.path());
        assert_eq!((report.received, report.waiting), (0, 2));
        fs::rename(&hidden, &blob).unwrap();
        let report = b.sync(cloud.path());
        assert_eq!((report.received, report.waiting), (2, 0));
    }

    #[test]
    fn half_written_lines_wait() {
        let cloud = tempfile::tempdir().unwrap();
        let (a, b) = (mac(), mac());
        a.sync(cloud.path());
        a.h().add(NewClip::text("whole")).unwrap();
        a.sync(cloud.path());
        let device = a.h().device_id().unwrap();
        let log = log_files(&cloud.path().join("devices").join(device))
            .unwrap()
            .pop()
            .unwrap();
        let mut f = OpenOptions::new().append(true).open(&log).unwrap();
        f.write_all(br#"{"op":"touch","hash":"#).unwrap();
        b.sync(cloud.path());
        assert_eq!(b.texts(), ["whole"]);
    }

    #[test]
    fn old_clips_from_elsewhere_respect_retention() {
        let cloud = tempfile::tempdir().unwrap();
        let (a, b) = (mac(), mac());
        a.h().add(NewClip::text("recent")).unwrap();
        a.sync(cloud.path());
        let week = Retention {
            max_age: Some(Duration::from_secs(7 * 24 * 3600)),
            max_bytes: None,
        };
        // Pretend B's clock is far ahead: A's clip is old to it.
        let op = Op::Add {
            hash: "old".into(),
            kind: "text".into(),
            text: "ancient".into(),
            ocr: String::new(),
            image: None,
            app: None,
            bundle: None,
            created: 0,
            used: 0,
            pinned: false,
        };
        assert_eq!(
            b.h().apply(&op, cloud.path(), week).unwrap(),
            Applied::Skipped
        );
        sync(&b.history, cloud.path(), "B", week).unwrap();
        assert_eq!(b.texts(), ["recent"]);
    }

    #[test]
    fn compaction_keeps_everything_for_new_macs() {
        let cloud = tempfile::tempdir().unwrap();
        let a = mac();
        a.sync(cloud.path());
        for i in 0..(LOG_LINES + 10) {
            a.h().add(NewClip::text(format!("clip {i}"))).unwrap();
        }
        let three = a.id("clip 3");
        a.h().delete(three).unwrap();
        a.sync(cloud.path());
        let own = cloud
            .path()
            .join("devices")
            .join(a.h().device_id().unwrap());
        assert!(log_files(&own).unwrap().len() >= 2, "logs rotate");

        let b = mac();
        b.sync(cloud.path());
        compact(&a.history, cloud.path(), &own).unwrap();
        let files = log_files(&own).unwrap();
        assert!(files.iter().all(|f| log_number(f) > 2), "old logs replaced");

        let c = mac();
        c.sync(cloud.path());
        b.sync(cloud.path());
        assert_eq!(c.texts().len(), LOG_LINES + 9);
        assert_eq!(b.texts().len(), LOG_LINES + 9);
        assert!(!c.texts().contains(&"clip 3".to_string()));
    }

    #[test]
    fn file_names_from_other_macs_cannot_escape() {
        let cloud = tempfile::tempdir().unwrap();
        let b = mac();
        fs::create_dir_all(cloud.path().join("blobs")).unwrap();
        // A tampered log: names meant to write outside the history folder.
        for (hash, ext) in [
            ("../../escaped", "png"),
            ("0123456789abcdef0123456789abcdef", "png/../../../escaped"),
            ("0123456789abcdef0123456789abcdef", "png\0"),
        ] {
            let op = Op::Add {
                hash: hash.into(),
                kind: "image".into(),
                text: String::new(),
                ocr: String::new(),
                image: Some(ext.into()),
                app: None,
                bundle: None,
                created: now_ms(),
                used: now_ms(),
                pinned: false,
            };
            assert_eq!(
                b.h().apply(&op, cloud.path(), KEEP).unwrap(),
                Applied::Skipped,
                "{hash}.{ext}"
            );
        }
        let dir = b.h().dir().to_path_buf();
        assert_eq!(fs::read_dir(dir.join("images")).unwrap().count(), 0);
        assert_eq!(b.h().stats().unwrap().0, 0);
    }

    #[test]
    fn safe_file_names() {
        use crate::history::safe_file_name as ok;
        assert!(ok("0123456789abcdef0123456789abcdef", "png"));
        assert!(ok("0123456789ABCDEF", "jpeg"));
        assert!(!ok("../x", "png"));
        assert!(!ok("0123456789abcdef", "png/.."));
        assert!(!ok("0123456789abcdef", ""));
        assert!(!ok("0123456789abcdef", "toolong"));
        assert!(!ok("abc", "png"), "too short");
    }

    #[test]
    fn ops_are_stable_json() {
        let op = Op::Touch {
            hash: "ab".into(),
            used: 5,
        };
        assert_eq!(
            serde_json::to_string(&op).unwrap(),
            r#"{"op":"touch","hash":"ab","used":5}"#
        );
        let add: Op = serde_json::from_str(
            r#"{"op":"add","hash":"h","kind":"text","text":"t","created":1,"used":2}"#,
        )
        .unwrap();
        assert!(matches!(
            add,
            Op::Add {
                pinned: false,
                image: None,
                ..
            }
        ));
    }
}
