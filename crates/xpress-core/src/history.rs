//! Clipboard and screenshot history: everything copied or captured, kept in a
//! local SQLite database with full-text search (FTS5) over the text, the text
//! recognised in images (see [`crate::ocr`]) and the source app.
//!
//! Layout of the history folder (by default `history/` in the config folder):
//!
//! ```text
//! history.db        clips + search index
//! images/<hash>.*   copied images and screenshots (PNG losslessly recompressed)
//! thumbs/<hash>.png previews, at most 320 px
//! ```
//!
//! The same content copied again isn't stored twice: it moves back to the top.
//!
//! A **multi-clip** collects several clips (text, images, files, …) to paste
//! together; its items stay clips of their own. **Categories** group clips by
//! hand or automatically, through a rule (source app, kind, words).

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::sync::Op;

use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

/// Text clips larger than this aren't recorded.
pub const MAX_TEXT_BYTES: usize = 5_000_000;
/// Longest side of a preview.
const THUMB_SIZE: u32 = 320;

/// What a clip holds, used for filtering and previews.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClipKind {
    Text,
    Link,
    Code,
    Color,
    Image,
    Screenshot,
    Files,
    /// Several clips to paste together.
    Multi,
}

impl ClipKind {
    pub const ALL: [ClipKind; 8] = [
        ClipKind::Text,
        ClipKind::Link,
        ClipKind::Code,
        ClipKind::Color,
        ClipKind::Image,
        ClipKind::Screenshot,
        ClipKind::Files,
        ClipKind::Multi,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ClipKind::Text => "text",
            ClipKind::Link => "link",
            ClipKind::Code => "code",
            ClipKind::Color => "color",
            ClipKind::Image => "image",
            ClipKind::Screenshot => "screenshot",
            ClipKind::Files => "files",
            ClipKind::Multi => "multi",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<ClipKind> {
        ClipKind::ALL.into_iter().find(|k| k.as_str() == s)
    }

    /// Plural display name, e.g. "Links".
    pub fn label(self) -> &'static str {
        match self {
            ClipKind::Text => "Text",
            ClipKind::Link => "Links",
            ClipKind::Code => "Code",
            ClipKind::Color => "Colours",
            ClipKind::Image => "Images",
            ClipKind::Screenshot => "Screenshots",
            ClipKind::Files => "Files",
            ClipKind::Multi => "Multi-clips",
        }
    }

    /// Whether the clip has a stored image.
    pub fn is_image(self) -> bool {
        matches!(self, ClipKind::Image | ClipKind::Screenshot)
    }
}

/// Something to record.
#[derive(Debug, Clone)]
pub struct NewClip {
    pub kind: ClipKind,
    /// The text; for files, one path per line.
    pub text: String,
    /// Encoded image bytes for image clips, and their file extension.
    pub image: Option<(Vec<u8>, String)>,
    /// The app that was in front when it was copied.
    pub source_app: Option<String>,
    pub source_bundle: Option<String>,
}

impl NewClip {
    /// Copied text, classified as text, link, code or colour.
    pub fn text(text: impl Into<String>) -> NewClip {
        let text = text.into();
        NewClip {
            kind: classify_text(&text),
            text,
            image: None,
            source_app: None,
            source_bundle: None,
        }
    }

    /// A copied image, as PNG bytes.
    pub fn image_png(png: Vec<u8>) -> NewClip {
        NewClip {
            kind: ClipKind::Image,
            text: String::new(),
            image: Some((png, "png".into())),
            source_app: None,
            source_bundle: None,
        }
    }

    /// A screenshot file (copied into the history; the original is untouched).
    pub fn screenshot(path: &Path) -> std::io::Result<NewClip> {
        let bytes = std::fs::read(path)?;
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("png")
            .to_ascii_lowercase();
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
        Ok(NewClip {
            kind: ClipKind::Screenshot,
            text: name.unwrap_or_default(),
            image: Some((bytes, ext)),
            source_app: Some("Screenshot".into()),
            source_bundle: Some("com.apple.screencaptureui".into()),
        })
    }

    /// Copied files or folders.
    pub fn files(paths: &[PathBuf]) -> NewClip {
        NewClip {
            kind: ClipKind::Files,
            text: paths
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join("\n"),
            image: None,
            source_app: None,
            source_bundle: None,
        }
    }

    pub fn from_app(mut self, name: Option<String>, bundle: Option<String>) -> NewClip {
        if name.is_some() {
            self.source_app = name;
            self.source_bundle = bundle;
        }
        self
    }
}

/// A recorded clip.
#[derive(Debug, Clone, PartialEq)]
pub struct Clip {
    pub id: i64,
    pub kind: ClipKind,
    pub text: String,
    /// Text recognised in the image (empty if none, or not yet recognised).
    pub ocr: String,
    pub image: Option<PathBuf>,
    pub thumb: Option<PathBuf>,
    /// Size of the stored content.
    pub bytes: u64,
    pub source_app: Option<String>,
    pub source_bundle: Option<String>,
    /// Unix milliseconds.
    pub created: i64,
    /// Unix milliseconds; when it was last copied (or copied back).
    pub last_used: i64,
    pub pinned: bool,
    /// The categories it's in.
    pub categories: Vec<i64>,
}

impl Clip {
    /// One line to show for it: the first line of text, the words found in
    /// an image (in quotes), or the files' names.
    pub fn title(&self) -> String {
        match self.kind {
            ClipKind::Files => {
                let names: Vec<String> = self
                    .paths()
                    .iter()
                    .map(|p| {
                        p.file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| p.display().to_string())
                    })
                    .collect();
                first_line(&names.join(", "), 140)
            }
            ClipKind::Image | ClipKind::Screenshot if !self.ocr.is_empty() => {
                format!("“{}”", first_line(&self.ocr, 140))
            }
            ClipKind::Screenshot if !self.text.is_empty() => first_line(&self.text, 140),
            ClipKind::Image | ClipKind::Screenshot => "Image".into(),
            ClipKind::Multi if self.text.is_empty() => "Multi-clip".into(),
            _ => first_line(&self.text, 140),
        }
    }

    /// The files of a [`ClipKind::Files`] clip.
    pub fn paths(&self) -> Vec<PathBuf> {
        if self.kind != ClipKind::Files {
            return Vec::new();
        }
        self.text.lines().map(PathBuf::from).collect()
    }
}

/// Whether `<hash>.<ext>` is a plain file name: a hex hash and a short
/// alphanumeric extension (names also arrive from other Macs when syncing).
pub(crate) fn safe_file_name(hash: &str, ext: &str) -> bool {
    (8..=64).contains(&hash.len())
        && hash.bytes().all(|b| b.is_ascii_hexdigit())
        && (1..=5).contains(&ext.len())
        && ext.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// The first non-empty line, at most `max` characters (then "…").
pub fn first_line(s: &str, max: usize) -> String {
    let line = s
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let mut out: String = line.chars().take(max).collect();
    if line.chars().count() > max {
        out.push('…');
    }
    out
}

/// "just now", "5 min ago", "3 h ago", "yesterday", "4 days ago".
pub fn ago(now_ms: i64, then_ms: i64) -> String {
    let secs = (now_ms - then_ms).max(0) / 1000;
    match secs {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", secs / 60),
        3600..86_400 => format!("{} h ago", secs / 3600),
        86_400..172_800 => "yesterday".into(),
        _ if secs < 60 * 86_400 => format!("{} days ago", secs / 86_400),
        _ => format!("{} months ago", secs / (30 * 86_400)),
    }
}

/// The result of [`History::add`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Added {
    pub id: i64,
    /// False when the same content was already there (it moved to the top).
    pub new: bool,
}

/// What to show.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Query {
    /// Words to find (prefix match, any order); empty for everything.
    pub text: String,
    pub kind: Option<ClipKind>,
    pub app: Option<String>,
    pub pinned_only: bool,
    pub category: Option<i64>,
    pub limit: usize,
}

/// A user-made group of clips, optionally filled automatically.
#[derive(Debug, Clone, PartialEq)]
pub struct Category {
    pub id: i64,
    pub name: String,
    pub color: [u8; 3],
    pub rule: Rule,
    /// How many clips are in it.
    pub count: usize,
}

/// Which new clips join a category by themselves. Every part that's set
/// must match; an empty rule means "only by hand".
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Rule {
    /// Copied in this app (case-insensitive).
    pub app: Option<String>,
    pub kind: Option<ClipKind>,
    /// Contains these words, in the text or in the text found in an image.
    pub contains: Option<String>,
}

impl Rule {
    pub fn is_empty(&self) -> bool {
        self.app.is_none() && self.kind.is_none() && self.contains.is_none()
    }

    pub fn matches(&self, clip: &Clip) -> bool {
        if self.is_empty() || clip.kind == ClipKind::Multi {
            return false;
        }
        let app_ok = self.app.as_ref().is_none_or(|want| {
            clip.source_app
                .as_ref()
                .is_some_and(|app| app.eq_ignore_ascii_case(want))
        });
        let kind_ok = self.kind.is_none_or(|k| k == clip.kind);
        let words_ok = self.contains.as_ref().is_none_or(|words| {
            let hay = format!("{}\n{}", clip.text, clip.ocr).to_lowercase();
            words
                .split_whitespace()
                .all(|w| hay.contains(&w.to_lowercase()))
        });
        app_ok && kind_ok && words_ok
    }
}

/// How long and how much to keep. Pinned clips are always kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Retention {
    pub max_age: Option<Duration>,
    pub max_bytes: Option<u64>,
}

pub struct History {
    pub(crate) conn: Connection,
    pub(crate) dir: PathBuf,
    /// Record changes for syncing (see [`crate::sync`]).
    journal: Cell<bool>,
    /// Applying changes from elsewhere (or rule-driven ones): don't record.
    quiet: Cell<bool>,
}

impl History {
    /// The default history folder: `history/` next to `config.json`.
    pub fn default_dir() -> Option<PathBuf> {
        if let Some(dir) = std::env::var_os("XPRESS_HISTORY_DIR").filter(|d| !d.is_empty()) {
            return Some(PathBuf::from(dir));
        }
        crate::config::Config::path().and_then(|p| p.parent().map(|d| d.join("history")))
    }

    /// What copying a clip puts on the clipboard: its text, image or files —
    /// for a multi-clip, all of its items.
    pub fn parts(&self, id: i64) -> rusqlite::Result<Vec<crate::clipboard::Part>> {
        use crate::clipboard::Part;
        let Some(clip) = self.get(id)? else {
            return Ok(Vec::new());
        };
        let clips = if clip.kind == ClipKind::Multi {
            self.items(id)?
        } else {
            vec![clip]
        };
        Ok(clips
            .into_iter()
            .flat_map(|c| match c.kind {
                ClipKind::Files => c.paths().into_iter().map(Part::File).collect(),
                ClipKind::Image | ClipKind::Screenshot => c
                    .image
                    .as_ref()
                    .and_then(|p| std::fs::read(p).ok())
                    .and_then(|bytes| {
                        if bytes.starts_with(b"\x89PNG") {
                            Some(bytes)
                        } else {
                            let img = image::load_from_memory(&bytes).ok()?;
                            let mut png = Vec::new();
                            img.write_to(
                                &mut std::io::Cursor::new(&mut png),
                                image::ImageFormat::Png,
                            )
                            .ok()?;
                            Some(png)
                        }
                    })
                    .map(Part::Png)
                    .into_iter()
                    .collect(),
                ClipKind::Multi => Vec::new(),
                _ => vec![Part::Text(c.text)],
            })
            .collect())
    }

    /// Open (creating if needed) the history in `dir`.
    pub fn open(dir: &Path) -> rusqlite::Result<History> {
        std::fs::create_dir_all(dir.join("images"))
            .and_then(|_| std::fs::create_dir_all(dir.join("thumbs")))
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        let conn = Connection::open(dir.join("history.db"))?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS clips (
                 id INTEGER PRIMARY KEY,
                 kind TEXT NOT NULL,
                 hash TEXT NOT NULL UNIQUE,
                 text TEXT NOT NULL DEFAULT '',
                 ocr TEXT NOT NULL DEFAULT '',
                 image TEXT,
                 thumb TEXT,
                 bytes INTEGER NOT NULL DEFAULT 0,
                 source_app TEXT,
                 source_bundle TEXT,
                 created INTEGER NOT NULL,
                 last_used INTEGER NOT NULL,
                 pinned INTEGER NOT NULL DEFAULT 0
             );
             CREATE INDEX IF NOT EXISTS clips_last_used ON clips(last_used DESC);
             CREATE VIRTUAL TABLE IF NOT EXISTS clips_fts USING fts5(
                 text, ocr, source_app,
                 content = 'clips', content_rowid = 'id',
                 tokenize = 'unicode61 remove_diacritics 2'
             );
             CREATE TRIGGER IF NOT EXISTS clips_ai AFTER INSERT ON clips BEGIN
                 INSERT INTO clips_fts(rowid, text, ocr, source_app)
                 VALUES (new.id, new.text, new.ocr, new.source_app);
             END;
             CREATE TRIGGER IF NOT EXISTS clips_ad AFTER DELETE ON clips BEGIN
                 INSERT INTO clips_fts(clips_fts, rowid, text, ocr, source_app)
                 VALUES ('delete', old.id, old.text, old.ocr, old.source_app);
             END;
             CREATE TRIGGER IF NOT EXISTS clips_au AFTER UPDATE OF text, ocr, source_app ON clips BEGIN
                 INSERT INTO clips_fts(clips_fts, rowid, text, ocr, source_app)
                 VALUES ('delete', old.id, old.text, old.ocr, old.source_app);
                 INSERT INTO clips_fts(rowid, text, ocr, source_app)
                 VALUES (new.id, new.text, new.ocr, new.source_app);
             END;
             CREATE TABLE IF NOT EXISTS clip_items (
                 parent_id INTEGER NOT NULL REFERENCES clips(id) ON DELETE CASCADE,
                 position INTEGER NOT NULL,
                 child_id INTEGER NOT NULL REFERENCES clips(id) ON DELETE CASCADE,
                 PRIMARY KEY (parent_id, position)
             );
             CREATE INDEX IF NOT EXISTS clip_items_child ON clip_items(child_id);
             CREATE TRIGGER IF NOT EXISTS clip_items_emptied AFTER DELETE ON clip_items
             WHEN NOT EXISTS (SELECT 1 FROM clip_items WHERE parent_id = old.parent_id)
             BEGIN
                 DELETE FROM clips WHERE id = old.parent_id;
             END;
             CREATE TABLE IF NOT EXISTS categories (
                 id INTEGER PRIMARY KEY,
                 name TEXT NOT NULL UNIQUE COLLATE NOCASE,
                 color TEXT NOT NULL,
                 rule_app TEXT,
                 rule_kind TEXT,
                 rule_text TEXT,
                 position INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE IF NOT EXISTS clip_categories (
                 clip_id INTEGER NOT NULL REFERENCES clips(id) ON DELETE CASCADE,
                 category_id INTEGER NOT NULL REFERENCES categories(id) ON DELETE CASCADE,
                 PRIMARY KEY (clip_id, category_id)
             );
             CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS outbox (id INTEGER PRIMARY KEY, op TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS tombstones (hash TEXT PRIMARY KEY, at INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS sync_progress (
                 device TEXT NOT NULL,
                 file TEXT NOT NULL,
                 lines INTEGER NOT NULL,
                 PRIMARY KEY (device, file)
             );
             PRAGMA user_version = 3;",
        )?;
        Ok(History {
            conn,
            dir: dir.to_path_buf(),
            journal: Cell::new(false),
            quiet: Cell::new(false),
        })
    }

    /// Record changes in the outbox so [`crate::sync`] can share them.
    pub fn set_journal(&self, on: bool) {
        self.journal.set(on);
    }

    pub fn journal(&self) -> bool {
        self.journal.get()
    }

    pub(crate) fn emit(&self, op: Op) -> rusqlite::Result<()> {
        if self.journal.get() && !self.quiet.get() {
            let json = serde_json::to_string(&op)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            self.conn
                .execute("INSERT INTO outbox (op) VALUES (?1)", [json])?;
        }
        Ok(())
    }

    /// Run `f` without recording its changes.
    pub(crate) fn quietly<T>(&self, f: impl FnOnce() -> T) -> T {
        let before = self.quiet.replace(true);
        let out = f();
        self.quiet.set(before);
        out
    }

    pub(crate) fn hash_of(&self, id: i64) -> rusqlite::Result<Option<String>> {
        self.conn
            .query_row("SELECT hash FROM clips WHERE id = ?1", [id], |r| r.get(0))
            .optional()
    }

    pub(crate) fn id_of(&self, hash: &str) -> rusqlite::Result<Option<i64>> {
        self.conn
            .query_row("SELECT id FROM clips WHERE hash = ?1", [hash], |r| r.get(0))
            .optional()
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Record a clip, or move the same content back to the top.
    pub fn add(&self, clip: NewClip) -> rusqlite::Result<Added> {
        self.add_at(clip, now_ms())
    }

    fn add_at(&self, clip: NewClip, at: i64) -> rusqlite::Result<Added> {
        if clip.text.len() > MAX_TEXT_BYTES {
            return Err(rusqlite::Error::ToSqlConversionFailure(
                "text too large for the history".into(),
            ));
        }
        let hash = content_hash(&clip);
        if let Some(id) = self
            .conn
            .query_row("SELECT id FROM clips WHERE hash = ?1", [&hash], |r| {
                r.get(0)
            })
            .optional()?
        {
            self.conn.execute(
                "UPDATE clips SET last_used = ?2,
                     source_app = COALESCE(?3, source_app),
                     source_bundle = COALESCE(?4, source_bundle)
                 WHERE id = ?1",
                params![id, at, clip.source_app, clip.source_bundle],
            )?;
            self.emit(Op::Touch { hash, used: at })?;
            return Ok(Added { id, new: false });
        }

        let (image, thumb, bytes) = match &clip.image {
            Some((data, ext)) => self.store_image(&hash, data, ext),
            None => (None, self.file_thumb(&clip, &hash), clip.text.len() as u64),
        };
        self.conn.execute(
            "INSERT INTO clips (kind, hash, text, image, thumb, bytes, source_app,
                                source_bundle, created, last_used)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
            params![
                clip.kind.as_str(),
                hash,
                clip.text,
                image,
                thumb,
                bytes as i64,
                clip.source_app,
                clip.source_bundle,
                at
            ],
        )?;
        let id = self.conn.last_insert_rowid();
        self.emit(Op::Add {
            hash,
            kind: clip.kind.as_str().into(),
            text: clip.text,
            ocr: String::new(),
            image: clip.image.map(|(_, ext)| ext),
            app: clip.source_app,
            bundle: clip.source_bundle,
            created: at,
            used: at,
            pinned: false,
        })?;
        self.apply_rules(id)?;
        Ok(Added { id, new: true })
    }

    // ---- Multi-clips --------------------------------------------------------

    /// Make a multi-clip of `ids`, in that order.
    pub fn combine(&self, ids: &[i64]) -> rusqlite::Result<i64> {
        if ids.is_empty() {
            return Err(rusqlite::Error::ToSqlConversionFailure(
                "nothing to combine".into(),
            ));
        }
        let at = now_ms();
        let hash = {
            let mut h = Sha256::new();
            h.update(b"multi");
            for id in ids {
                h.update(id.to_le_bytes());
            }
            h.update(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0)
                    .to_le_bytes(),
            );
            hex16(&h.finalize())
        };
        self.conn.execute(
            "INSERT INTO clips (kind, hash, created, last_used) VALUES ('multi', ?1, ?2, ?2)",
            params![hash, at],
        )?;
        let multi = self.conn.last_insert_rowid();
        for (pos, id) in ids.iter().enumerate() {
            self.conn.execute(
                "INSERT INTO clip_items (parent_id, position, child_id) VALUES (?1, ?2, ?3)",
                params![multi, pos as i64, id],
            )?;
        }
        self.refresh_multi(multi)?;
        self.emit_multi(multi)?;
        Ok(multi)
    }

    /// Share a multi-clip's current items.
    fn emit_multi(&self, multi: i64) -> rusqlite::Result<()> {
        if !self.journal.get() || self.quiet.get() {
            return Ok(());
        }
        let Some(clip) = self.get(multi)? else {
            return Ok(());
        };
        let hash = self.hash_of(multi)?.unwrap_or_default();
        let items = self
            .items(multi)?
            .into_iter()
            .filter_map(|c| self.hash_of(c.id).ok().flatten())
            .collect();
        self.emit(Op::Multi {
            hash,
            items,
            created: clip.created,
            used: clip.last_used,
            pinned: clip.pinned,
        })
    }

    /// Add `clip` to the end of `multi`, or start a new multi-clip with it
    /// when there's none (or it's gone). Returns the multi-clip.
    pub fn append_to(&self, multi: Option<i64>, clip: i64) -> rusqlite::Result<i64> {
        let existing = match multi {
            Some(id) => self.get(id)?.filter(|c| c.kind == ClipKind::Multi),
            None => None,
        };
        let Some(multi) = existing else {
            return self.combine(&[clip]);
        };
        let last: Option<(i64, i64)> = self
            .conn
            .query_row(
                "SELECT position, child_id FROM clip_items WHERE parent_id = ?1
                 ORDER BY position DESC LIMIT 1",
                [multi.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if last.map(|(_, child)| child) != Some(clip) {
            self.conn.execute(
                "INSERT INTO clip_items (parent_id, position, child_id) VALUES (?1, ?2, ?3)",
                params![multi.id, last.map_or(0, |(pos, _)| pos + 1), clip],
            )?;
            self.refresh_multi(multi.id)?;
        }
        self.touch(multi.id)?;
        self.emit_multi(multi.id)?;
        Ok(multi.id)
    }

    /// The clips in a multi-clip, in order.
    pub fn items(&self, multi: i64) -> rusqlite::Result<Vec<Clip>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLUMNS} FROM clip_items JOIN clips ON clips.id = clip_items.child_id
             WHERE clip_items.parent_id = ?1 ORDER BY clip_items.position"
        ))?;
        let rows = stmt.query_map([multi], |r| self.row(r))?;
        rows.collect()
    }

    /// Keep a multi-clip's searchable text in step with its items.
    pub(crate) fn refresh_multi(&self, multi: i64) -> rusqlite::Result<()> {
        let text: Vec<String> = self
            .items(multi)?
            .iter()
            .map(|c| {
                if c.kind.is_image() {
                    c.ocr.clone()
                } else {
                    c.text.chars().take(2000).collect()
                }
            })
            .filter(|t| !t.is_empty())
            .collect();
        self.conn
            .execute(
                "UPDATE clips SET text = ?2 WHERE id = ?1",
                params![multi, text.join("\n")],
            )
            .map(|_| ())
    }

    fn parents_of(&self, clip: i64) -> rusqlite::Result<Vec<i64>> {
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT parent_id FROM clip_items WHERE child_id = ?1")?;
        let rows = stmt.query_map([clip], |r| r.get(0))?;
        rows.collect()
    }

    // ---- Categories ---------------------------------------------------------

    pub fn categories(&self) -> rusqlite::Result<Vec<Category>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, color, rule_app, rule_kind, rule_text,
                    (SELECT COUNT(*) FROM clip_categories WHERE category_id = categories.id)
             FROM categories ORDER BY position, name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], |r| {
            let color: String = r.get(2)?;
            let kind: Option<String> = r.get(4)?;
            Ok(Category {
                id: r.get(0)?,
                name: r.get(1)?,
                color: parse_color(&color).map_or([122, 162, 247], |[r, g, b, _]| [r, g, b]),
                rule: Rule {
                    app: r.get(3)?,
                    kind: kind.as_deref().and_then(ClipKind::from_str),
                    contains: r.get(5)?,
                },
                count: r.get::<_, i64>(6)? as usize,
            })
        })?;
        rows.collect()
    }

    /// Create a category (`id: None`) or change one. Clips already in the
    /// history that match the rule join it. Returns its id.
    pub fn save_category(
        &self,
        id: Option<i64>,
        name: &str,
        color: [u8; 3],
        rule: &Rule,
    ) -> rusqlite::Result<i64> {
        let name = name.trim();
        if name.is_empty() {
            return Err(rusqlite::Error::ToSqlConversionFailure(
                "a category needs a name".into(),
            ));
        }
        let clean = |s: &Option<String>| {
            s.as_ref()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let color = format!("#{:02x}{:02x}{:02x}", color[0], color[1], color[2]);
        let previous: Option<String> = match id {
            Some(id) => self
                .conn
                .query_row("SELECT name FROM categories WHERE id = ?1", [id], |r| {
                    r.get(0)
                })
                .optional()?,
            None => None,
        };
        self.emit(Op::Category {
            name: name.to_string(),
            previous: previous.filter(|p| p != name),
            color: color.clone(),
            app: clean(&rule.app),
            kind: rule.kind.map(|k| k.as_str().to_string()),
            contains: clean(&rule.contains),
        })?;
        let values = params![
            name,
            color,
            clean(&rule.app),
            rule.kind.map(|k| k.as_str()),
            clean(&rule.contains),
            id
        ];
        let id = match id {
            Some(id) => {
                self.conn.execute(
                    "UPDATE categories SET name = ?1, color = ?2, rule_app = ?3,
                         rule_kind = ?4, rule_text = ?5 WHERE id = ?6",
                    values,
                )?;
                id
            }
            None => {
                self.conn.execute(
                    "INSERT INTO categories (name, color, rule_app, rule_kind, rule_text, position)
                     VALUES (?1, ?2, ?3, ?4, ?5,
                             COALESCE(?6, (SELECT COUNT(*) FROM categories)))",
                    values,
                )?;
                self.conn.last_insert_rowid()
            }
        };
        if let Some(category) = self.categories()?.into_iter().find(|c| c.id == id) {
            if !category.rule.is_empty() {
                let everything = Query {
                    limit: usize::MAX >> 1,
                    ..Default::default()
                };
                // Other Macs apply the rule themselves.
                self.quietly(|| -> rusqlite::Result<()> {
                    for clip in self.search(&everything)? {
                        if category.rule.matches(&clip) {
                            self.set_category(clip.id, id, true)?;
                        }
                    }
                    Ok(())
                })?;
            }
        }
        Ok(id)
    }

    pub fn delete_category(&self, id: i64) -> rusqlite::Result<()> {
        let name: Option<String> = self
            .conn
            .query_row("SELECT name FROM categories WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .optional()?;
        if let Some(name) = name {
            self.emit(Op::CategoryDelete { name })?;
        }
        self.conn
            .execute("DELETE FROM categories WHERE id = ?1", [id])
            .map(|_| ())
    }

    /// Put a clip in a category, or take it out.
    pub fn set_category(&self, clip: i64, category: i64, on: bool) -> rusqlite::Result<()> {
        let sql = if on {
            "INSERT OR IGNORE INTO clip_categories (clip_id, category_id) VALUES (?1, ?2)"
        } else {
            "DELETE FROM clip_categories WHERE clip_id = ?1 AND category_id = ?2"
        };
        self.conn.execute(sql, params![clip, category])?;
        if self.journal.get() && !self.quiet.get() {
            let name: Option<String> = self
                .conn
                .query_row(
                    "SELECT name FROM categories WHERE id = ?1",
                    [category],
                    |r| r.get(0),
                )
                .optional()?;
            if let (Some(hash), Some(category)) = (self.hash_of(clip)?, name) {
                self.emit(Op::Categorize { hash, category, on })?;
            }
        }
        Ok(())
    }

    /// Add a clip to every category whose rule it matches.
    pub(crate) fn apply_rules(&self, clip: i64) -> rusqlite::Result<()> {
        let Some(clip) = self.get(clip)? else {
            return Ok(());
        };
        // Every Mac applies its own rules.
        self.quietly(|| {
            for category in self.categories()? {
                if category.rule.matches(&clip) {
                    self.set_category(clip.id, category.id, true)?;
                }
            }
            Ok(())
        })
    }

    /// Write the image (PNGs losslessly recompressed) and its preview.
    /// Returns relative paths and the stored size.
    pub(crate) fn store_image(
        &self,
        hash: &str,
        data: &[u8],
        ext: &str,
    ) -> (Option<String>, Option<String>, u64) {
        if !safe_file_name(hash, ext) {
            return (None, None, 0);
        }
        let stored = if ext == "png" {
            oxipng::optimize_from_memory(data, &oxipng::Options::from_preset(2))
                .ok()
                .filter(|o| o.len() < data.len())
                .unwrap_or_else(|| data.to_vec())
        } else {
            data.to_vec()
        };
        let image = format!("images/{hash}.{ext}");
        if std::fs::write(self.dir.join(&image), &stored).is_err() {
            return (None, None, 0);
        }
        let thumb = make_thumb(data).and_then(|png| {
            let rel = format!("thumbs/{hash}.png");
            std::fs::write(self.dir.join(&rel), png).ok().map(|_| rel)
        });
        (Some(image), thumb, stored.len() as u64)
    }

    /// A preview for a single copied image file.
    fn file_thumb(&self, clip: &NewClip, hash: &str) -> Option<String> {
        if clip.kind != ClipKind::Files || clip.text.lines().count() != 1 {
            return None;
        }
        let path = Path::new(clip.text.trim());
        crate::filetype::classify(path).filter(|k| *k == crate::filetype::MediaKind::Image)?;
        let png = make_thumb(&std::fs::read(path).ok()?)?;
        let rel = format!("thumbs/{hash}.png");
        std::fs::write(self.dir.join(&rel), png).ok().map(|_| rel)
    }

    /// Store the text recognised in a clip's image.
    pub fn set_ocr(&self, id: i64, text: &str) -> rusqlite::Result<()> {
        self.conn
            .execute("UPDATE clips SET ocr = ?2 WHERE id = ?1", params![id, text])?;
        if let Some(hash) = self.hash_of(id)? {
            self.emit(Op::Ocr {
                hash,
                text: text.to_string(),
            })?;
        }
        // Rules on words can match now.
        self.apply_rules(id)?;
        for parent in self.parents_of(id)? {
            self.refresh_multi(parent)?;
        }
        Ok(())
    }

    /// Mark a clip as just used (copied back), moving it to the top.
    pub fn touch(&self, id: i64) -> rusqlite::Result<()> {
        let at = now_ms();
        self.conn.execute(
            "UPDATE clips SET last_used = ?2 WHERE id = ?1",
            params![id, at],
        )?;
        if let Some(hash) = self.hash_of(id)? {
            self.emit(Op::Touch { hash, used: at })?;
        }
        Ok(())
    }

    pub fn set_pinned(&self, id: i64, pinned: bool) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE clips SET pinned = ?2 WHERE id = ?1",
            params![id, pinned],
        )?;
        if let Some(hash) = self.hash_of(id)? {
            self.emit(Op::Pin { hash, pinned })?;
        }
        Ok(())
    }

    pub fn get(&self, id: i64) -> rusqlite::Result<Option<Clip>> {
        self.conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM clips WHERE id = ?1"),
                [id],
                |r| self.row(r),
            )
            .optional()
    }

    /// Clips matching `query`, most recently used first.
    pub fn search(&self, query: &Query) -> rusqlite::Result<Vec<Clip>> {
        let limit = if query.limit == 0 { 200 } else { query.limit } as i64;
        let fts = fts_query(&query.text);
        let sql = format!(
            "SELECT {COLUMNS} FROM clips
             WHERE (?1 IS NULL OR id IN (SELECT rowid FROM clips_fts WHERE clips_fts MATCH ?1))
               AND (?2 IS NULL OR kind = ?2)
               AND (?3 IS NULL OR source_app = ?3)
               AND (?4 = 0 OR pinned = 1)
               AND (?6 IS NULL OR id IN
                    (SELECT clip_id FROM clip_categories WHERE category_id = ?6))
             ORDER BY last_used DESC, id DESC
             LIMIT ?5"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(
            params![
                fts,
                query.kind.map(|k| k.as_str()),
                query.app,
                query.pinned_only,
                limit,
                query.category
            ],
            |r| self.row(r),
        )?;
        rows.collect()
    }

    /// Source apps with how many clips came from each, most first.
    pub fn apps(&self) -> rusqlite::Result<Vec<(String, usize)>> {
        let mut stmt = self.conn.prepare(
            "SELECT source_app, COUNT(*) FROM clips WHERE source_app IS NOT NULL
             GROUP BY source_app ORDER BY COUNT(*) DESC, source_app",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get::<_, i64>(1)? as usize)))?;
        rows.collect()
    }

    /// The apps clips came from, with their bundle ids and how many.
    pub fn app_sources(&self) -> rusqlite::Result<Vec<(String, Option<String>, usize)>> {
        let mut stmt = self.conn.prepare(
            "SELECT source_app, MAX(source_bundle), COUNT(*) FROM clips
             WHERE source_app IS NOT NULL AND kind != 'screenshot'
             GROUP BY source_app ORDER BY COUNT(*) DESC, source_app",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? as usize))
        })?;
        rows.collect()
    }

    /// The clips copied in an app: by bundle id when known, else by name.
    fn from_app_filter(bundle: Option<&str>) -> &'static str {
        if bundle.is_some() {
            "(source_bundle = ?2 OR (source_bundle IS NULL AND source_app = ?1 COLLATE NOCASE))"
        } else {
            "source_app = ?1 COLLATE NOCASE"
        }
    }

    pub fn count_from_app(&self, name: &str, bundle: Option<&str>) -> rusqlite::Result<usize> {
        let sql = format!(
            "SELECT COUNT(*) FROM clips WHERE kind != 'multi' AND {}",
            Self::from_app_filter(bundle)
        );
        let n: i64 = match bundle {
            Some(b) => self.conn.query_row(&sql, params![name, b], |r| r.get(0))?,
            None => self.conn.query_row(&sql, params![name], |r| r.get(0))?,
        };
        Ok(n as usize)
    }

    /// Delete every clip copied in an app (on every Mac, when syncing).
    pub fn delete_from_app(&self, name: &str, bundle: Option<&str>) -> rusqlite::Result<usize> {
        let filter = format!("kind != 'multi' AND {}", Self::from_app_filter(bundle));
        let ids = match bundle {
            Some(b) => self.ids_where(&filter, params![name, b])?,
            None => self.ids_where(&filter, params![name])?,
        };
        for id in &ids {
            if self.get(*id)?.is_some() {
                self.delete(*id)?;
            }
        }
        Ok(ids.len())
    }

    /// Number of clips and the bytes they take up.
    pub fn stats(&self) -> rusqlite::Result<(usize, u64)> {
        self.conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(bytes), 0) FROM clips",
            [],
            |r| Ok((r.get::<_, i64>(0)? as usize, r.get::<_, i64>(1)? as u64)),
        )
    }

    /// Delete a clip (on every Mac, when syncing).
    pub fn delete(&self, id: i64) -> rusqlite::Result<()> {
        if let Some(hash) = self.hash_of(id)? {
            let at = now_ms();
            if self.journal.get() || self.quiet.get() {
                self.conn.execute(
                    "INSERT OR REPLACE INTO tombstones (hash, at) VALUES (?1, ?2)",
                    params![hash, at],
                )?;
            }
            self.emit(Op::Delete { hash, at })?;
        }
        self.remove(id)
    }

    /// Delete a clip here only (retention).
    fn remove(&self, id: i64) -> rusqlite::Result<()> {
        let parents = self.parents_of(id)?;
        let files: Option<(Option<String>, Option<String>)> = self
            .conn
            .query_row("SELECT image, thumb FROM clips WHERE id = ?1", [id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        self.conn.execute("DELETE FROM clips WHERE id = ?1", [id])?;
        if let Some((image, thumb)) = files {
            for rel in [image, thumb].into_iter().flatten() {
                let _ = std::fs::remove_file(self.dir.join(rel));
            }
        }
        // Multi-clips it was in (an emptied one removes itself).
        for parent in parents {
            if self.get(parent)?.is_some() {
                self.refresh_multi(parent)?;
            }
        }
        Ok(())
    }

    /// Delete every clip (except pinned ones, with `keep_pinned`).
    pub fn clear(&self, keep_pinned: bool) -> rusqlite::Result<usize> {
        let filter = if keep_pinned {
            format!("pinned = 0 AND {NOT_IN_PINNED_MULTI}")
        } else {
            "1".to_string()
        };
        let ids = self.ids_where(&filter, [])?;
        let mut removed = 0;
        for id in ids {
            if self.get(id)?.is_some() {
                self.delete(id)?;
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// Drop unpinned clips past the retention limits; returns how many.
    pub fn prune(&self, retention: Retention) -> rusqlite::Result<usize> {
        self.prune_at(retention, now_ms())
    }

    fn prune_at(&self, retention: Retention, now: i64) -> rusqlite::Result<usize> {
        let mut removed = 0;
        if let Some(age) = retention.max_age {
            let cutoff = now - age.as_millis() as i64;
            let filter = format!("pinned = 0 AND last_used < ?1 AND {NOT_IN_MULTI}");
            for id in self.ids_where(&filter, [cutoff])? {
                if self.get(id)?.is_some() {
                    self.remove(id)?;
                    removed += 1;
                }
            }
        }
        if let Some(max) = retention.max_bytes {
            let (_, mut total) = self.stats()?;
            if total > max {
                let mut stmt = self.conn.prepare(&format!(
                    "SELECT id, bytes FROM clips WHERE pinned = 0 AND {NOT_IN_MULTI}
                     ORDER BY last_used ASC, id ASC"
                ))?;
                let oldest: Vec<(i64, i64)> = stmt
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<rusqlite::Result<_>>()?;
                for (id, bytes) in oldest {
                    if total <= max {
                        break;
                    }
                    self.remove(id)?;
                    total = total.saturating_sub(bytes as u64);
                    removed += 1;
                }
            }
        }
        Ok(removed)
    }

    fn ids_where<P: rusqlite::Params>(&self, filter: &str, p: P) -> rusqlite::Result<Vec<i64>> {
        let mut stmt = self
            .conn
            .prepare(&format!("SELECT id FROM clips WHERE {filter}"))?;
        let rows = stmt.query_map(p, |r| r.get(0))?;
        rows.collect()
    }

    pub(crate) fn row(&self, r: &rusqlite::Row) -> rusqlite::Result<Clip> {
        let kind: String = r.get(1)?;
        let abs = |rel: Option<String>| rel.map(|p| self.dir.join(p));
        Ok(Clip {
            id: r.get(0)?,
            kind: ClipKind::from_str(&kind).unwrap_or(ClipKind::Text),
            text: r.get(2)?,
            ocr: r.get(3)?,
            image: abs(r.get(4)?),
            thumb: abs(r.get(5)?),
            bytes: r.get::<_, i64>(6)? as u64,
            source_app: r.get(7)?,
            source_bundle: r.get(8)?,
            created: r.get(9)?,
            last_used: r.get(10)?,
            pinned: r.get(11)?,
            categories: r
                .get::<_, Option<String>>(12)?
                .map(|ids| ids.split(',').filter_map(|i| i.parse().ok()).collect())
                .unwrap_or_default(),
        })
    }
}

pub(crate) const COLUMNS: &str = "clips.id, kind, text, ocr, image, thumb, bytes, source_app, \
     source_bundle, created, last_used, pinned, \
     (SELECT GROUP_CONCAT(category_id) FROM clip_categories WHERE clip_id = clips.id)";

/// Items of a multi-clip are kept as long as it is.
const NOT_IN_MULTI: &str = "id NOT IN (SELECT child_id FROM clip_items)";
const NOT_IN_PINNED_MULTI: &str = "id NOT IN (SELECT child_id FROM clip_items \
     JOIN clips p ON p.id = clip_items.parent_id WHERE p.pinned = 1)";

fn hex16(digest: &[u8]) -> String {
    digest[..16].iter().map(|b| format!("{b:02x}")).collect()
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn content_hash(clip: &NewClip) -> String {
    let mut h = Sha256::new();
    match &clip.image {
        // By pixels, so a picture stored as a screenshot and the same
        // picture copied back from the history are one clip.
        Some((data, _)) => match image::load_from_memory(data) {
            Ok(img) => {
                let rgba = img.to_rgba8();
                h.update(rgba.width().to_le_bytes());
                h.update(rgba.height().to_le_bytes());
                h.update(rgba.as_raw());
            }
            Err(_) => h.update(data),
        },
        None => {
            h.update(clip.kind.as_str());
            h.update([0]);
            h.update(clip.text.as_bytes());
        }
    }
    hex16(&h.finalize())
}

/// Every word as a quoted prefix term, so `inv 47` finds "Invoice 4711" and
/// punctuation can't break the FTS syntax. `None` for an empty query.
fn fts_query(text: &str) -> Option<String> {
    let terms: Vec<String> = text
        .split_whitespace()
        .map(|w| format!("\"{}\"*", w.replace('"', "\"\"")))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" "))
}

fn make_thumb(data: &[u8]) -> Option<Vec<u8>> {
    let img = image::load_from_memory(data).ok()?;
    let thumb = if img.width() > THUMB_SIZE || img.height() > THUMB_SIZE {
        img.thumbnail(THUMB_SIZE, THUMB_SIZE)
    } else {
        img
    };
    let mut out = Vec::new();
    thumb
        .to_rgba8()
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .ok()?;
    Some(out)
}

/// Text, link, code or colour.
pub fn classify_text(text: &str) -> ClipKind {
    let t = text.trim();
    if parse_color(t).is_some() {
        return ClipKind::Color;
    }
    if is_link(t) {
        return ClipKind::Link;
    }
    if looks_like_code(text) {
        return ClipKind::Code;
    }
    ClipKind::Text
}

fn is_link(t: &str) -> bool {
    if t.is_empty() || t.contains(char::is_whitespace) {
        return false;
    }
    let lower = t.to_ascii_lowercase();
    ["http://", "https://", "ftp://", "mailto:", "file://"]
        .iter()
        .any(|p| lower.starts_with(p) && lower.len() > p.len())
        || (lower.starts_with("www.") && lower[4..].contains('.'))
}

fn looks_like_code(text: &str) -> bool {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let mut score = 0;
    let ends = lines
        .iter()
        .filter(|l| {
            let l = l.trim_end();
            l.ends_with(';') || l.ends_with('{') || l.ends_with('}') || l.ends_with("=>")
        })
        .count();
    if ends >= 2 || (lines.len() == 1 && ends == 1 && text.contains('(')) {
        score += 2;
    }
    if lines.len() >= 2
        && lines
            .iter()
            .filter(|l| l.starts_with("    ") || l.starts_with('\t'))
            .count()
            >= 1
    {
        score += 1;
    }
    let keywords = [
        "fn ",
        "let ",
        "const ",
        "def ",
        "function ",
        "class ",
        "import ",
        "#include",
        "return ",
        "pub ",
        "var ",
        "SELECT ",
        "</",
        "=> ",
        "func ",
        "package ",
    ];
    if keywords.iter().any(|k| text.contains(k)) {
        score += 1;
    }
    score >= 2
}

/// Parse a CSS-style colour: `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`,
/// `rgb()`/`rgba()` and `hsl()`/`hsla()`. Returns RGBA.
pub fn parse_color(s: &str) -> Option<[u8; 4]> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let digit = |i: usize| u8::from_str_radix(&hex[i..i + 1], 16).ok().map(|v| v * 17);
        let pair = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        return match hex.len() {
            3 => Some([digit(0)?, digit(1)?, digit(2)?, 255]),
            4 => Some([digit(0)?, digit(1)?, digit(2)?, digit(3)?]),
            6 => Some([pair(0)?, pair(2)?, pair(4)?, 255]),
            8 => Some([pair(0)?, pair(2)?, pair(4)?, pair(6)?]),
            _ => None,
        };
    }
    let lower = s.to_ascii_lowercase();
    let (func, rest) = lower.split_once('(')?;
    let args: Vec<&str> = rest
        .strip_suffix(')')?
        .split([',', ' ', '/'])
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .collect();
    let alpha = |a: Option<&&str>| -> Option<u8> {
        match a {
            None => Some(255),
            Some(a) => {
                let v = match a.strip_suffix('%') {
                    Some(p) => p.parse::<f64>().ok()? / 100.0,
                    None => a.parse::<f64>().ok()?,
                };
                (0.0..=1.0).contains(&v).then(|| (v * 255.0).round() as u8)
            }
        }
    };
    match func {
        "rgb" | "rgba" if args.len() == 3 || args.len() == 4 => {
            let channel = |a: &str| -> Option<u8> {
                let v = match a.strip_suffix('%') {
                    Some(p) => p.parse::<f64>().ok()? * 2.55,
                    None => a.parse::<f64>().ok()?,
                };
                (0.0..=255.0).contains(&v).then(|| v.round() as u8)
            };
            Some([
                channel(args[0])?,
                channel(args[1])?,
                channel(args[2])?,
                alpha(args.get(3))?,
            ])
        }
        "hsl" | "hsla" if args.len() == 3 || args.len() == 4 => {
            let h = args[0]
                .trim_end_matches("deg")
                .parse::<f64>()
                .ok()?
                .rem_euclid(360.0);
            let pct = |a: &str| -> Option<f64> {
                let v = a.strip_suffix('%')?.parse::<f64>().ok()? / 100.0;
                (0.0..=1.0).contains(&v).then_some(v)
            };
            let (s, l) = (pct(args[1])?, pct(args[2])?);
            let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
            let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
            let m = l - c / 2.0;
            let (r, g, b) = match (h / 60.0) as u32 {
                0 => (c, x, 0.0),
                1 => (x, c, 0.0),
                2 => (0.0, c, x),
                3 => (0.0, x, c),
                4 => (x, 0.0, c),
                _ => (c, 0.0, x),
            };
            let to = |v: f64| ((v + m) * 255.0).round() as u8;
            Some([to(r), to(g), to(b), alpha(args.get(3))?])
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history() -> (tempfile::TempDir, History) {
        let dir = tempfile::tempdir().unwrap();
        let h = History::open(dir.path()).unwrap();
        (dir, h)
    }

    fn png(w: u32, h: u32, shade: u8) -> Vec<u8> {
        let mut out = Vec::new();
        image::RgbImage::from_pixel(w, h, image::Rgb([shade, 100, 50]))
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    fn texts(clips: &[Clip]) -> Vec<&str> {
        clips.iter().map(|c| c.text.as_str()).collect()
    }

    #[test]
    fn records_newest_first_and_dedupes() {
        let (_d, h) = history();
        let a = h.add_at(NewClip::text("first"), 1).unwrap();
        h.add_at(NewClip::text("second"), 2).unwrap();
        assert!(a.new);
        let again = h.add_at(NewClip::text("first"), 3).unwrap();
        assert_eq!(
            again,
            Added {
                id: a.id,
                new: false
            }
        );
        let all = h.search(&Query::default()).unwrap();
        assert_eq!(texts(&all), ["first", "second"]);
        assert_eq!(h.stats().unwrap().0, 2);
    }

    #[test]
    fn full_text_search_with_prefixes_kinds_and_apps() {
        let (_d, h) = history();
        h.add(NewClip::text("Invoice 4711 is paid").from_app(Some("Mail".into()), None))
            .unwrap();
        h.add(NewClip::text("https://example.com/docs").from_app(Some("Safari".into()), None))
            .unwrap();
        h.add(NewClip::text("Café au lait")).unwrap();
        let find = |text: &str| {
            texts(
                &h.search(&Query {
                    text: text.into(),
                    ..Default::default()
                })
                .unwrap(),
            )
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>()
        };
        assert_eq!(find("inv 47"), ["Invoice 4711 is paid"]);
        assert_eq!(find("PAID invoice"), ["Invoice 4711 is paid"]);
        assert_eq!(find("example"), ["https://example.com/docs"]);
        assert_eq!(find("cafe"), ["Café au lait"], "diacritics are ignored");
        assert_eq!(find("safari"), ["https://example.com/docs"], "by app name");
        assert!(find("nothing-like-this").is_empty());
        assert_eq!(find("\"quoted"), Vec::<String>::new(), "odd input is safe");

        let links = h
            .search(&Query {
                kind: Some(ClipKind::Link),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(texts(&links), ["https://example.com/docs"]);
        let mail = h
            .search(&Query {
                app: Some("Mail".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(texts(&mail), ["Invoice 4711 is paid"]);
        assert_eq!(
            h.apps().unwrap(),
            [("Mail".to_string(), 1), ("Safari".to_string(), 1)]
        );
    }

    #[test]
    fn images_are_stored_with_previews_and_searchable_by_ocr() {
        let (dir, h) = history();
        let added = h.add(NewClip::image_png(png(800, 400, 10))).unwrap();
        let clip = h.get(added.id).unwrap().unwrap();
        assert_eq!(clip.kind, ClipKind::Image);
        let image = clip.image.clone().unwrap();
        assert!(image.starts_with(dir.path().join("images")));
        assert!(image.exists());
        let thumb = image::open(clip.thumb.clone().unwrap()).unwrap();
        assert_eq!((thumb.width(), thumb.height()), (320, 160));
        assert_eq!(clip.bytes, std::fs::metadata(&image).unwrap().len());

        h.set_ocr(added.id, "error: file not found").unwrap();
        let found = h
            .search(&Query {
                text: "file not".into(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].ocr, "error: file not found");

        // The same image again is the same clip, however it's encoded.
        assert!(!h.add(NewClip::image_png(png(800, 400, 10))).unwrap().new);
        let stored = std::fs::read(&image).unwrap();
        assert!(!h.add(NewClip::image_png(stored)).unwrap().new);
        assert!(h.add(NewClip::image_png(png(800, 400, 11))).unwrap().new);
        h.delete(added.id).unwrap();
        assert!(!image.exists());
        assert!(h.get(added.id).unwrap().is_none());
    }

    #[test]
    fn screenshots_and_files() {
        let (dir, h) = history();
        let shot = dir.path().join("Screenshot 2026-10-04 at 12.00.00.png");
        std::fs::write(&shot, png(100, 50, 20)).unwrap();
        let s = h.add(NewClip::screenshot(&shot).unwrap()).unwrap();
        let clip = h.get(s.id).unwrap().unwrap();
        assert_eq!(clip.kind, ClipKind::Screenshot);
        assert_eq!(clip.source_app.as_deref(), Some("Screenshot"));
        assert!(shot.exists(), "the original stays");

        let files = h
            .add(NewClip::files(&[
                shot.clone(),
                PathBuf::from("/tmp/notes.txt"),
            ]))
            .unwrap();
        let clip = h.get(files.id).unwrap().unwrap();
        assert_eq!(
            clip.paths(),
            [shot.clone(), PathBuf::from("/tmp/notes.txt")]
        );
        assert!(clip.thumb.is_none());
        let single = h.add(NewClip::files(std::slice::from_ref(&shot))).unwrap();
        assert!(h.get(single.id).unwrap().unwrap().thumb.is_some());
    }

    #[test]
    fn retention_keeps_pinned_clips() {
        let (_d, h) = history();
        let day = 24 * 60 * 60 * 1000;
        let old = h.add_at(NewClip::text("old"), 0).unwrap();
        let pinned = h.add_at(NewClip::text("pinned"), 0).unwrap();
        h.set_pinned(pinned.id, true).unwrap();
        h.add_at(NewClip::text("recent"), 9 * day).unwrap();
        let removed = h
            .prune_at(
                Retention {
                    max_age: Some(Duration::from_millis(7 * day as u64)),
                    max_bytes: None,
                },
                10 * day,
            )
            .unwrap();
        assert_eq!(removed, 1);
        assert!(h.get(old.id).unwrap().is_none());
        assert_eq!(
            texts(&h.search(&Query::default()).unwrap()),
            ["recent", "pinned"]
        );
        let pinned_only = h
            .search(&Query {
                pinned_only: true,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(texts(&pinned_only), ["pinned"]);
    }

    #[test]
    fn retention_by_size_drops_the_oldest_first() {
        let (_d, h) = history();
        for (i, t) in ["aaaa", "bbbb", "cccc"].iter().enumerate() {
            h.add_at(NewClip::text(*t), i as i64).unwrap();
        }
        let removed = h
            .prune(Retention {
                max_age: None,
                max_bytes: Some(8),
            })
            .unwrap();
        assert_eq!(removed, 1);
        assert_eq!(
            texts(&h.search(&Query::default()).unwrap()),
            ["cccc", "bbbb"]
        );
        assert_eq!(h.clear(false).unwrap(), 2);
        assert_eq!(h.stats().unwrap(), (0, 0));
    }

    #[test]
    fn touch_moves_a_clip_to_the_top_and_reopen_keeps_everything() {
        let dir = tempfile::tempdir().unwrap();
        let first = {
            let h = History::open(dir.path()).unwrap();
            let first = h.add_at(NewClip::text("one"), 1).unwrap();
            h.add_at(NewClip::text("two"), 2).unwrap();
            h.touch(first.id).unwrap();
            first
        };
        let h = History::open(dir.path()).unwrap();
        let all = h.search(&Query::default()).unwrap();
        assert_eq!(all[0].id, first.id);
        assert_eq!(texts(&all), ["one", "two"]);
    }

    #[test]
    fn too_much_text_is_refused() {
        let (_d, h) = history();
        assert!(h
            .add(NewClip::text("x".repeat(MAX_TEXT_BYTES + 1)))
            .is_err());
    }

    #[test]
    fn multi_clips_combine_search_and_survive_pruning() {
        let (_d, h) = history();
        let day = 24 * 60 * 60 * 1000;
        let a = h.add_at(NewClip::text("first part"), 0).unwrap().id;
        let img = h.add_at(NewClip::image_png(png(40, 20, 1)), 0).unwrap().id;
        h.set_ocr(img, "receipt total").unwrap();
        let multi = h.combine(&[a, img]).unwrap();

        let clip = h.get(multi).unwrap().unwrap();
        assert_eq!(clip.kind, ClipKind::Multi);
        assert_eq!(clip.text, "first part\nreceipt total");
        assert_eq!(
            h.items(multi)
                .unwrap()
                .iter()
                .map(|c| c.id)
                .collect::<Vec<_>>(),
            [a, img]
        );
        let found = h
            .search(&Query {
                text: "receipt".into(),
                kind: Some(ClipKind::Multi),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(found.len(), 1);

        // Old items are kept as long as their multi-clip is.
        let a_week_later = Retention {
            max_age: Some(Duration::from_millis(day as u64)),
            max_bytes: None,
        };
        assert_eq!(h.prune_at(a_week_later, now_ms() + 7 * day).unwrap(), 1);
        assert!(
            h.get(multi).unwrap().is_none(),
            "the multi-clip was old too"
        );
        assert_eq!(h.prune_at(a_week_later, now_ms() + 7 * day).unwrap(), 2);

        let a = h.add_at(NewClip::text("kept"), 0).unwrap().id;
        let multi = h.combine(&[a]).unwrap();
        assert_eq!(h.prune_at(a_week_later, now_ms()).unwrap(), 0);
        assert!(h.get(a).unwrap().is_some());
        h.set_pinned(multi, true).unwrap();
        assert_eq!(
            h.clear(true).unwrap(),
            0,
            "items of a pinned multi-clip stay"
        );
    }

    #[test]
    fn deleting_items_updates_and_finally_removes_a_multi_clip() {
        let (_d, h) = history();
        let a = h.add(NewClip::text("alpha")).unwrap().id;
        let b = h.add(NewClip::text("beta")).unwrap().id;
        let multi = h.combine(&[a, b]).unwrap();
        h.delete(a).unwrap();
        assert_eq!(h.get(multi).unwrap().unwrap().text, "beta");
        h.delete(b).unwrap();
        assert!(h.get(multi).unwrap().is_none(), "an empty multi-clip goes");
        // Deleting a multi-clip keeps its items.
        let c = h.add(NewClip::text("gamma")).unwrap().id;
        let m = h.combine(&[c]).unwrap();
        h.delete(m).unwrap();
        assert!(h.get(c).unwrap().is_some());
    }

    #[test]
    fn collecting_appends_to_one_multi_clip() {
        let (_d, h) = history();
        let a = h.add(NewClip::text("one")).unwrap().id;
        let b = h.add(NewClip::text("two")).unwrap().id;
        let m = h.append_to(None, a).unwrap();
        assert_eq!(h.append_to(Some(m), b).unwrap(), m);
        assert_eq!(
            h.append_to(Some(m), b).unwrap(),
            m,
            "same clip twice in a row: once"
        );
        assert_eq!(h.items(m).unwrap().len(), 2);
        h.delete(m).unwrap();
        let fresh = h.append_to(Some(m), a).unwrap();
        assert_eq!(
            h.items(fresh).unwrap().len(),
            1,
            "a gone multi-clip starts a new one"
        );
    }

    #[test]
    fn categories_by_hand_and_by_rule() {
        let (_d, h) = history();
        let mail = h
            .add(NewClip::text("Invoice 4711").from_app(Some("Mail".into()), None))
            .unwrap()
            .id;
        let note = h.add(NewClip::text("groceries: milk")).unwrap().id;

        // A rule fills the category with matching clips already there…
        let invoices = h
            .save_category(
                None,
                "Invoices",
                [255, 158, 100],
                &Rule {
                    contains: Some("invoice".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(h.get(mail).unwrap().unwrap().categories, [invoices]);
        // …and with new ones, also when the words are found in an image later.
        let later = h.add(NewClip::text("invoice 4712 paid")).unwrap().id;
        assert_eq!(h.get(later).unwrap().unwrap().categories, [invoices]);
        let img = h.add(NewClip::image_png(png(30, 30, 9))).unwrap().id;
        assert!(h.get(img).unwrap().unwrap().categories.is_empty());
        h.set_ocr(img, "INVOICE from ACME").unwrap();
        assert_eq!(h.get(img).unwrap().unwrap().categories, [invoices]);

        // By hand.
        let personal = h
            .save_category(None, "Personal", [10, 200, 10], &Rule::default())
            .unwrap();
        h.set_category(note, personal, true).unwrap();
        let in_personal = h
            .search(&Query {
                category: Some(personal),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(texts(&in_personal), ["groceries: milk"]);

        let cats = h.categories().unwrap();
        assert_eq!(
            cats.iter()
                .map(|c| (c.name.as_str(), c.count))
                .collect::<Vec<_>>(),
            [("Invoices", 3), ("Personal", 1)]
        );
        assert_eq!(cats[0].color, [255, 158, 100]);
        assert!(
            h.save_category(None, "invoices", [0, 0, 0], &Rule::default())
                .is_err(),
            "names are unique"
        );
        assert!(h
            .save_category(None, "  ", [0, 0, 0], &Rule::default())
            .is_err());

        // Rename, change the rule; delete.
        h.save_category(
            Some(personal),
            "Home",
            [1, 2, 3],
            &Rule {
                app: Some("mail".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let home = h
            .categories()
            .unwrap()
            .into_iter()
            .find(|c| c.id == personal)
            .unwrap();
        assert_eq!(home.name, "Home");
        assert_eq!(home.count, 2, "the note, and the Mail clip by the new rule");
        h.delete_category(invoices).unwrap();
        assert_eq!(h.get(mail).unwrap().unwrap().categories, [personal]);
        h.set_category(note, personal, false).unwrap();
        assert!(h.get(note).unwrap().unwrap().categories.is_empty());
    }

    #[test]
    fn rules() {
        let clip = Clip {
            id: 1,
            kind: ClipKind::Link,
            text: "https://github.com/kwhorne".into(),
            ocr: String::new(),
            image: None,
            thumb: None,
            bytes: 0,
            source_app: Some("Safari".into()),
            source_bundle: None,
            created: 0,
            last_used: 0,
            pinned: false,
            categories: Vec::new(),
        };
        let rule = |app: Option<&str>, kind: Option<ClipKind>, words: Option<&str>| Rule {
            app: app.map(Into::into),
            kind,
            contains: words.map(Into::into),
        };
        assert!(
            !rule(None, None, None).matches(&clip),
            "empty: by hand only"
        );
        assert!(rule(Some("safari"), None, None).matches(&clip));
        assert!(rule(None, Some(ClipKind::Link), Some("GITHUB kwhorne")).matches(&clip));
        assert!(!rule(Some("Safari"), Some(ClipKind::Text), None).matches(&clip));
        assert!(!rule(None, None, Some("gitlab")).matches(&clip));
    }

    #[test]
    fn what_copying_a_clip_puts_on_the_clipboard() {
        use crate::clipboard::Part;
        let (_d, h) = history();
        let text = h.add(NewClip::text("hello")).unwrap().id;
        let img = h.add(NewClip::image_png(png(4, 4, 1))).unwrap().id;
        let files = h.add(NewClip::files(&["/a.txt".into()])).unwrap().id;
        assert_eq!(h.parts(text).unwrap(), [Part::Text("hello".into())]);
        assert!(matches!(h.parts(img).unwrap()[..], [Part::Png(_)]));
        assert_eq!(h.parts(files).unwrap(), [Part::File("/a.txt".into())]);
        let multi = h.combine(&[files, text]).unwrap();
        assert_eq!(
            h.parts(multi).unwrap(),
            [Part::File("/a.txt".into()), Part::Text("hello".into())]
        );
        assert!(h.parts(9999).unwrap().is_empty());
    }

    #[test]
    fn titles_and_times() {
        let mut clip = h_clip(ClipKind::Text, "\n  first line \nsecond");
        assert_eq!(clip.title(), "first line");
        clip = h_clip(ClipKind::Files, "/a/report.pdf\n/b/photo.jpg");
        assert_eq!(clip.title(), "report.pdf, photo.jpg");
        clip = h_clip(ClipKind::Image, "");
        assert_eq!(clip.title(), "Image");
        clip.ocr = "Invoice 4711\npaid".into();
        assert_eq!(clip.title(), "“Invoice 4711”");
        assert!(h_clip(ClipKind::Text, &"x".repeat(500))
            .title()
            .ends_with('…'));
        let min = 60_000;
        assert_eq!(ago(10 * min, 10 * min), "just now");
        assert_eq!(ago(10 * min, 5 * min), "5 min ago");
        assert_eq!(ago(200 * min, 20 * min), "3 h ago");
        assert_eq!(ago(2000 * min, 20 * min), "yesterday");
        assert_eq!(ago(10_000 * min, 20 * min), "6 days ago");
        assert_eq!(ago(200_000 * min, 0), "4 months ago");
    }

    fn h_clip(kind: ClipKind, text: &str) -> Clip {
        Clip {
            id: 1,
            kind,
            text: text.into(),
            ocr: String::new(),
            image: None,
            thumb: None,
            bytes: 0,
            source_app: None,
            source_bundle: None,
            created: 0,
            last_used: 0,
            pinned: false,
            categories: Vec::new(),
        }
    }

    #[test]
    fn clips_by_source_app() {
        let (_d, h) = history();
        let app =
            |name: &str, bundle: Option<&str>| (Some(name.to_string()), bundle.map(str::to_string));
        let (n, b) = app("Bank", Some("com.bank.app"));
        h.add(NewClip::text("account 1234").from_app(n, b)).unwrap();
        let (n, b) = app("Bank", Some("com.bank.app"));
        h.add(NewClip::text("account 5678").from_app(n, b)).unwrap();
        let (n, b) = app("Notes", None);
        h.add(NewClip::text("shopping").from_app(n, b)).unwrap();
        let note = h.add(NewClip::text("unrelated")).unwrap().id;
        let both = h.combine(&[note]).unwrap();

        let sources = h.app_sources().unwrap();
        assert_eq!(sources[0], ("Bank".into(), Some("com.bank.app".into()), 2));
        assert_eq!(sources[1], ("Notes".into(), None, 1));
        assert_eq!(h.count_from_app("Bank", Some("com.bank.app")).unwrap(), 2);
        assert_eq!(
            h.count_from_app("notes", None).unwrap(),
            1,
            "names ignore case"
        );

        assert_eq!(h.delete_from_app("Bank", Some("com.bank.app")).unwrap(), 2);
        assert_eq!(h.count_from_app("Bank", Some("com.bank.app")).unwrap(), 0);
        assert!(h.get(note).unwrap().is_some() && h.get(both).unwrap().is_some());
        assert_eq!(
            h.stats().unwrap().0,
            3,
            "notes, unrelated and the multi-clip remain"
        );
    }

    #[test]
    fn classifies_text() {
        assert_eq!(classify_text("hello world"), ClipKind::Text);
        assert_eq!(classify_text("https://kwhorne.com/x?y=1"), ClipKind::Link);
        assert_eq!(classify_text("  www.example.org "), ClipKind::Link);
        assert_eq!(classify_text("mailto:kh@example.com"), ClipKind::Link);
        assert_eq!(classify_text("see https://x.y for more"), ClipKind::Text);
        assert_eq!(classify_text("#7aa2f7"), ClipKind::Color);
        assert_eq!(classify_text("rgb(10, 20, 30)"), ClipKind::Color);
        assert_eq!(
            classify_text("fn main() {\n    println!(\"hi\");\n}"),
            ClipKind::Code
        );
        assert_eq!(classify_text("let x = compute(1);"), ClipKind::Code);
        assert_eq!(
            classify_text("Dear Ann,\nthanks for the call.\nBest, Kim"),
            ClipKind::Text
        );
        assert_eq!(classify_text("#hashtag"), ClipKind::Text);
    }

    #[test]
    fn parses_colours() {
        assert_eq!(parse_color("#fff"), Some([255, 255, 255, 255]));
        assert_eq!(parse_color("#7aa2f7"), Some([0x7a, 0xa2, 0xf7, 255]));
        assert_eq!(parse_color("#7aa2f780"), Some([0x7a, 0xa2, 0xf7, 0x80]));
        assert_eq!(parse_color("rgba(255, 0, 0, 0.5)"), Some([255, 0, 0, 128]));
        assert_eq!(parse_color("rgb(100% 0% 0%)"), Some([255, 0, 0, 255]));
        assert_eq!(parse_color("hsl(120, 100%, 50%)"), Some([0, 255, 0, 255]));
        assert_eq!(
            parse_color("hsl(240deg 100% 50% / 50%)"),
            Some([0, 0, 255, 128])
        );
        assert_eq!(parse_color("#12345"), None);
        assert_eq!(parse_color("rgb(300, 0, 0)"), None);
        assert_eq!(parse_color("blue"), None);
    }
}
