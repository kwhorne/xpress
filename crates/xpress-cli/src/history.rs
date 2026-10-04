//! `xpress history`: the desktop app's clipboard history from the terminal —
//! search it, print or copy a clip, pin and delete. `--json` makes it easy to
//! use from Raycast, Alfred, Shortcuts or scripts.

use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use serde_json::{json, Value};
use xpress_core::history::{ago, now_ms, Clip, ClipKind, History, Query};

use crate::render;

#[derive(Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct HistoryArgs {
    #[command(subcommand)]
    cmd: Option<HistoryCmd>,
    #[command(flatten)]
    search: SearchArgs,
}

#[derive(Subcommand)]
enum HistoryCmd {
    /// Search the history, most recently used first (the default).
    Search(SearchArgs),
    /// Print a clip: its text or file paths; for an image, the image's path.
    Show {
        id: i64,
        /// For an image: print the text found in it instead of its path.
        #[arg(long)]
        text: bool,
    },
    /// Put a clip back on the clipboard (macOS), ready to paste.
    Copy { id: i64 },
    /// Keep a clip forever.
    Pin { id: i64 },
    /// Let a clip expire again.
    Unpin { id: i64 },
    /// Delete clips.
    Delete {
        #[arg(required = true)]
        ids: Vec<i64>,
    },
    /// List the categories.
    Categories {
        #[arg(long)]
        json: bool,
    },
}

#[derive(Args, Default)]
struct SearchArgs {
    /// Words to find (in text, file names, words in images and app names).
    words: Vec<String>,
    /// Only this kind: text, link, code, color, image, screenshot, files, multi.
    #[arg(long, value_parser = parse_kind)]
    kind: Option<ClipKind>,
    /// Only clips copied in this app.
    #[arg(long)]
    app: Option<String>,
    /// Only clips in this category.
    #[arg(long)]
    category: Option<String>,
    /// Only pinned clips.
    #[arg(long)]
    pinned: bool,
    /// How many to show.
    #[arg(short = 'n', long, default_value_t = 20)]
    limit: usize,
    /// Machine-readable output.
    #[arg(long)]
    json: bool,
}

fn parse_kind(s: &str) -> Result<ClipKind, String> {
    let s = s.to_ascii_lowercase();
    let s = match s.as_str() {
        "colour" | "colours" | "colors" => "color",
        "links" => "link",
        "images" => "image",
        "screenshots" => "screenshot",
        "file" => "files",
        "multi-clip" | "multiclip" => "multi",
        other => other,
    };
    ClipKind::from_str(s).ok_or_else(|| {
        "expected text, link, code, color, image, screenshot, files or multi".to_string()
    })
}

pub fn run(args: HistoryArgs) -> Result<()> {
    let history = open()?;
    match args.cmd {
        None => search(&history, &args.search),
        Some(HistoryCmd::Search(search_args)) => search(&history, &search_args),
        Some(HistoryCmd::Show { id, text }) => {
            let clip = get(&history, id)?;
            print!("{}", show(&history, &clip, text)?);
            Ok(())
        }
        Some(HistoryCmd::Copy { id }) => {
            let clip = get(&history, id)?;
            let parts = history.parts(id)?;
            if !xpress_core::clipboard::write(&parts) {
                bail!("could not copy — putting clips on the clipboard needs macOS (try `xpress history show {id}`)");
            }
            history.touch(id)?;
            println!("{} copied: {}", render::CHECK, clip.title());
            Ok(())
        }
        Some(HistoryCmd::Pin { id }) => pin(&history, id, true),
        Some(HistoryCmd::Unpin { id }) => pin(&history, id, false),
        Some(HistoryCmd::Delete { ids }) => {
            for id in &ids {
                get(&history, *id)?;
            }
            for id in &ids {
                history.delete(*id)?;
            }
            println!(
                "{} deleted {} clip{}",
                render::CHECK,
                ids.len(),
                if ids.len() == 1 { "" } else { "s" }
            );
            Ok(())
        }
        Some(HistoryCmd::Categories { json }) => {
            let categories = history.categories()?;
            if json {
                let list: Vec<Value> = categories
                    .iter()
                    .map(|c| {
                        json!({
                            "id": c.id,
                            "name": c.name,
                            "color": format!("#{:02x}{:02x}{:02x}", c.color[0], c.color[1], c.color[2]),
                            "count": c.count,
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&list)?);
            } else if categories.is_empty() {
                println!("No categories yet — make them in the xpress app (History → + Category).");
            } else {
                for c in categories {
                    println!("{:>5}  {}", c.count, c.name);
                }
            }
            Ok(())
        }
    }
}

/// The app's history, if it has one. Changes are shared with other Macs
/// when the app syncs.
fn open() -> Result<History> {
    let dir = History::default_dir().context("no config folder")?;
    if !dir.join("history.db").exists() {
        bail!(
            "no clipboard history yet — turn it on in the xpress app (History → Turn on clipboard history)"
        );
    }
    let history = History::open(&dir)?;
    history.set_journal(app_syncs());
    Ok(history)
}

/// Whether the app has "Sync with iCloud" on (its gui.json).
fn app_syncs() -> bool {
    let Some(path) = xpress_core::config::Config::path().map(|p| p.with_file_name("gui.json"))
    else {
        return false;
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    let settings: Value = serde_json::from_str(&text).unwrap_or_default();
    settings["historySync"].as_bool() == Some(true)
        && settings["historyEnabled"].as_bool() == Some(true)
}

fn get(history: &History, id: i64) -> Result<Clip> {
    history
        .get(id)?
        .with_context(|| format!("there's no clip {id} (see `xpress history`)"))
}

fn pin(history: &History, id: i64, on: bool) -> Result<()> {
    let clip = get(history, id)?;
    history.set_pinned(id, on)?;
    println!(
        "{} {}: {}",
        render::CHECK,
        if on { "pinned" } else { "unpinned" },
        clip.title()
    );
    Ok(())
}

fn search(history: &History, args: &SearchArgs) -> Result<()> {
    let category = match &args.category {
        Some(name) => Some(
            history
                .categories()?
                .into_iter()
                .find(|c| c.name.eq_ignore_ascii_case(name))
                .with_context(|| format!("there's no category “{name}”"))?
                .id,
        ),
        None => None,
    };
    let clips = history.search(&Query {
        text: args.words.join(" "),
        kind: args.kind,
        app: args.app.clone(),
        pinned_only: args.pinned,
        category,
        limit: args.limit.max(1),
    })?;
    if args.json {
        let names = history.categories()?;
        let list: Vec<Value> = clips.iter().map(|c| to_json(c, &names)).collect();
        println!("{}", serde_json::to_string_pretty(&list)?);
    } else if clips.is_empty() {
        println!("No clips match.");
    } else {
        let now = now_ms();
        for clip in &clips {
            println!("{}", line(clip, now));
        }
    }
    Ok(())
}

/// `   42  ★ link    2 min ago    Safari       https://…`
fn line(clip: &Clip, now: i64) -> String {
    let app = clip
        .source_app
        .as_deref()
        .map(|a| xpress_core::history::first_line(a, 12))
        .unwrap_or_default();
    format!(
        "{:>6}  {} {:<10} {:<12} {:<13} {}",
        clip.id,
        if clip.pinned { "★" } else { " " },
        clip.kind.as_str(),
        ago(now, clip.last_used),
        app,
        xpress_core::history::first_line(&clip.title(), 90)
    )
}

fn to_json(clip: &Clip, categories: &[xpress_core::history::Category]) -> Value {
    json!({
        "id": clip.id,
        "kind": clip.kind.as_str(),
        "title": clip.title(),
        "text": clip.text,
        "ocr": clip.ocr,
        "paths": clip.paths(),
        "image": clip.image,
        "app": clip.source_app,
        "created": clip.created,
        "lastUsed": clip.last_used,
        "pinned": clip.pinned,
        "categories": clip
            .categories
            .iter()
            .filter_map(|id| categories.iter().find(|c| c.id == *id).map(|c| c.name.clone()))
            .collect::<Vec<_>>(),
    })
}

/// What `show` prints.
fn show(history: &History, clip: &Clip, text: bool) -> Result<String> {
    if clip.kind.is_image() {
        let out = if text {
            clip.ocr.clone()
        } else {
            clip.image
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        };
        return Ok(format!("{out}\n"));
    }
    let composed = xpress_core::clipboard::compose(&history.parts(clip.id)?);
    let mut out = if composed.files.is_empty() {
        composed.text
    } else {
        composed
            .files
            .iter()
            .map(|f| f.display().to_string())
            .collect::<Vec<_>>()
            .join("\n")
    };
    if !out.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use xpress_core::history::NewClip;

    #[test]
    fn kinds_by_any_reasonable_name() {
        assert_eq!(parse_kind("Links"), Ok(ClipKind::Link));
        assert_eq!(parse_kind("colour"), Ok(ClipKind::Color));
        assert_eq!(parse_kind("files"), Ok(ClipKind::Files));
        assert_eq!(parse_kind("multi-clip"), Ok(ClipKind::Multi));
        assert!(parse_kind("sound").is_err());
    }

    #[test]
    fn lines_and_shown_content() {
        let dir = tempfile::tempdir().unwrap();
        let h = History::open(dir.path()).unwrap();
        let link = h
            .add(NewClip::text("https://kwhorne.com").from_app(Some("Safari".into()), None))
            .unwrap()
            .id;
        h.set_pinned(link, true).unwrap();
        let clip = h.get(link).unwrap().unwrap();
        let l = line(&clip, clip.last_used);
        assert!(l.contains("★ link"), "{l}");
        assert!(
            l.contains("just now") && l.contains("Safari") && l.ends_with("https://kwhorne.com")
        );

        let files = h
            .add(NewClip::files(&["/a.txt".into(), "/b.txt".into()]))
            .unwrap()
            .id;
        let multi = h.combine(&[link, files]).unwrap();
        assert_eq!(
            show(&h, &h.get(files).unwrap().unwrap(), false).unwrap(),
            "/a.txt\n/b.txt\n"
        );
        assert_eq!(
            show(&h, &h.get(multi).unwrap().unwrap(), false).unwrap(),
            "https://kwhorne.com\n/a.txt\n/b.txt\n"
        );
        let json = to_json(&clip, &[]);
        assert_eq!(json["kind"], "link");
        assert_eq!(json["pinned"], true);
        assert_eq!(json["app"], "Safari");
    }
}
