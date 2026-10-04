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

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
}

impl ClipKind {
    pub const ALL: [ClipKind; 7] = [
        ClipKind::Text,
        ClipKind::Link,
        ClipKind::Code,
        ClipKind::Color,
        ClipKind::Image,
        ClipKind::Screenshot,
        ClipKind::Files,
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
}

impl Clip {
    /// The files of a [`ClipKind::Files`] clip.
    pub fn paths(&self) -> Vec<PathBuf> {
        if self.kind != ClipKind::Files {
            return Vec::new();
        }
        self.text.lines().map(PathBuf::from).collect()
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
    pub limit: usize,
}

/// How long and how much to keep. Pinned clips are always kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Retention {
    pub max_age: Option<Duration>,
    pub max_bytes: Option<u64>,
}

pub struct History {
    conn: Connection,
    dir: PathBuf,
}

impl History {
    /// The default history folder: `history/` next to `config.json`.
    pub fn default_dir() -> Option<PathBuf> {
        crate::config::Config::path().and_then(|p| p.parent().map(|d| d.join("history")))
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
             PRAGMA user_version = 1;",
        )?;
        Ok(History {
            conn,
            dir: dir.to_path_buf(),
        })
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
        Ok(Added {
            id: self.conn.last_insert_rowid(),
            new: true,
        })
    }

    /// Write the image (PNGs losslessly recompressed) and its preview.
    /// Returns relative paths and the stored size.
    fn store_image(
        &self,
        hash: &str,
        data: &[u8],
        ext: &str,
    ) -> (Option<String>, Option<String>, u64) {
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
            .execute("UPDATE clips SET ocr = ?2 WHERE id = ?1", params![id, text])
            .map(|_| ())
    }

    /// Mark a clip as just used (copied back), moving it to the top.
    pub fn touch(&self, id: i64) -> rusqlite::Result<()> {
        self.conn
            .execute(
                "UPDATE clips SET last_used = ?2 WHERE id = ?1",
                params![id, now_ms()],
            )
            .map(|_| ())
    }

    pub fn set_pinned(&self, id: i64, pinned: bool) -> rusqlite::Result<()> {
        self.conn
            .execute(
                "UPDATE clips SET pinned = ?2 WHERE id = ?1",
                params![id, pinned],
            )
            .map(|_| ())
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
                limit
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

    /// Number of clips and the bytes they take up.
    pub fn stats(&self) -> rusqlite::Result<(usize, u64)> {
        self.conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(bytes), 0) FROM clips",
            [],
            |r| Ok((r.get::<_, i64>(0)? as usize, r.get::<_, i64>(1)? as u64)),
        )
    }

    pub fn delete(&self, id: i64) -> rusqlite::Result<()> {
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
        Ok(())
    }

    /// Delete every clip (except pinned ones, with `keep_pinned`).
    pub fn clear(&self, keep_pinned: bool) -> rusqlite::Result<usize> {
        let ids = self.ids_where(if keep_pinned { "pinned = 0" } else { "1" }, [])?;
        for id in &ids {
            self.delete(*id)?;
        }
        Ok(ids.len())
    }

    /// Drop unpinned clips past the retention limits; returns how many.
    pub fn prune(&self, retention: Retention) -> rusqlite::Result<usize> {
        self.prune_at(retention, now_ms())
    }

    fn prune_at(&self, retention: Retention, now: i64) -> rusqlite::Result<usize> {
        let mut removed = 0;
        if let Some(age) = retention.max_age {
            let cutoff = now - age.as_millis() as i64;
            for id in self.ids_where("pinned = 0 AND last_used < ?1", [cutoff])? {
                self.delete(id)?;
                removed += 1;
            }
        }
        if let Some(max) = retention.max_bytes {
            let (_, mut total) = self.stats()?;
            if total > max {
                let mut stmt = self.conn.prepare(
                    "SELECT id, bytes FROM clips WHERE pinned = 0 ORDER BY last_used ASC, id ASC",
                )?;
                let oldest: Vec<(i64, i64)> = stmt
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<rusqlite::Result<_>>()?;
                for (id, bytes) in oldest {
                    if total <= max {
                        break;
                    }
                    self.delete(id)?;
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

    fn row(&self, r: &rusqlite::Row) -> rusqlite::Result<Clip> {
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
        })
    }
}

const COLUMNS: &str = "id, kind, text, ocr, image, thumb, bytes, source_app, source_bundle, \
                       created, last_used, pinned";

fn now_ms() -> i64 {
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
    h.finalize()[..16]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
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
