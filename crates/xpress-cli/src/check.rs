//! `xpress check`: a read-only guard for CI. Fails (exit code 1) when media
//! files are over a size limit, or when optimising them would still save a
//! meaningful share — i.e. unoptimised assets slipped into the repo.
//!
//! Files are never modified: each one is optimised to a temporary file just to
//! measure what it could shrink to.

use std::path::{Path, PathBuf};

use xpress_core::audio::AudioFormat;
use xpress_core::filetype::MediaKind;
use xpress_core::result::{OptimisationResult, OptimiseError, OptimiseOptions};

use crate::render::{self, OutputMode};

/// What `check` enforces.
pub struct Limits {
    /// Largest acceptable file size in bytes.
    pub max_size: Option<u64>,
    /// Flag files that optimising would shrink by at least this percentage.
    pub min_savings: f64,
}

/// One file's verdict.
struct Verdict {
    path: PathBuf,
    size: u64,
    optimised: u64,
    problems: Vec<String>,
}

impl Verdict {
    fn savings(&self) -> f64 {
        if self.size == 0 {
            return 0.0;
        }
        (self.size.saturating_sub(self.optimised)) as f64 / self.size as f64 * 100.0
    }
}

/// Whether any component of `path` is one of the excluded directory names.
pub fn excluded(path: &Path, exclude: &[String]) -> bool {
    path.components()
        .any(|c| exclude.iter().any(|e| c.as_os_str() == e.as_str()))
}

/// Optimise `path` into a throwaway temp file to see how small it could get.
///
/// Images are measured against a perceptual target (`quality`, SSIMULACRA2)
/// instead of a compression factor: re-encoding a lossy file always "saves"
/// something by discarding more detail, so an already-optimised JPEG would be
/// flagged forever. What matters is how much smaller it could be *without a
/// visible change*.
pub fn dry_run(
    path: &Path,
    options: &OptimiseOptions,
    quality: f64,
) -> Result<OptimisationResult, OptimiseError> {
    let tmp = tempfile::TempDir::new()?;
    let out = tmp.path().join(xpress_core::result::file_name_lossy(path));
    let opts = OptimiseOptions {
        output: Some(out),
        backup: false,
        allow_larger: false,
        ..options.clone()
    };
    if xpress_core::filetype::classify(path) == Some(MediaKind::Image) {
        xpress_core::quality::optimise_to_quality(path, quality, &opts)
    } else {
        xpress_core::optimise_file(path, &opts, AudioFormat::SameAsInput, None)
    }
}

/// Judge the dry-run results; print them; return the number of files with
/// problems.
pub fn report(
    results: &[(PathBuf, Result<OptimisationResult, OptimiseError>)],
    limits: &Limits,
    mode: OutputMode,
) -> usize {
    let mut verdicts = Vec::new();
    let mut errors = Vec::new();
    for (path, res) in results {
        match res {
            Ok(r) => {
                let optimised = if r.improved() { r.new_size } else { r.old_size };
                let mut v = Verdict {
                    path: path.clone(),
                    size: r.old_size,
                    optimised,
                    problems: Vec::new(),
                };
                if let Some(max) = limits.max_size {
                    if v.size > max {
                        v.problems.push(format!(
                            "{} is over the {} limit",
                            render::human_size(v.size),
                            render::human_size(max)
                        ));
                    }
                }
                if v.savings() >= limits.min_savings {
                    v.problems.push(format!(
                        "could be {:.0}% smaller ({} {} {})",
                        v.savings(),
                        render::human_size(v.size),
                        render::ARROW,
                        render::human_size(v.optimised)
                    ));
                }
                verdicts.push(v);
            }
            Err(e) => errors.push((path.clone(), e.to_string())),
        }
    }
    let failing = verdicts.iter().filter(|v| !v.problems.is_empty()).count() + errors.len();

    if mode == OutputMode::Json {
        let mut items: Vec<serde_json::Value> = verdicts
            .iter()
            .map(|v| {
                serde_json::json!({
                    "path": v.path.display().to_string(),
                    "ok": v.problems.is_empty(),
                    "size": v.size,
                    "optimised_size": v.optimised,
                    "savings_percent": (v.savings() * 10.0).round() / 10.0,
                    "problems": v.problems,
                })
            })
            .collect();
        items.extend(errors.iter().map(|(p, e)| {
            serde_json::json!({ "path": p.display().to_string(), "ok": false, "error": e })
        }));
        println!(
            "{}",
            serde_json::to_string_pretty(&items).unwrap_or_else(|_| "[]".into())
        );
        return failing;
    }

    for v in verdicts.iter().filter(|v| !v.problems.is_empty()) {
        println!(
            "{} {}  {}",
            render::ERROR_X,
            v.path.display(),
            v.problems.join("; ")
        );
    }
    for (p, e) in &errors {
        println!("{} {}  {e}", render::ERROR_X, p.display());
    }
    let checked = verdicts.len() + errors.len();
    if failing == 0 {
        println!("{} {checked} files checked — all optimised", render::CHECK);
    } else {
        let savable: u64 = verdicts
            .iter()
            .filter(|v| !v.problems.is_empty())
            .map(|v| v.size.saturating_sub(v.optimised))
            .sum();
        println!(
            "\n{checked} files checked, {failing} with problems — `xpress optimise` would save {}",
            render::human_size(savable)
        );
    }
    failing
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(old: u64, new: u64) -> Result<OptimisationResult, OptimiseError> {
        Ok(OptimisationResult {
            kind: xpress_core::filetype::MediaKind::Image,
            source: PathBuf::from("a.png"),
            output: PathBuf::from("a.png"),
            backup: None,
            old_size: old,
            new_size: new,
            aggressive: false,
            cached: false,
            score: None,
        })
    }

    #[test]
    fn flags_oversized_and_unoptimised_files() {
        let limits = Limits {
            max_size: Some(500_000),
            min_savings: 10.0,
        };
        let results = vec![
            (PathBuf::from("ok.png"), result(100_000, 95_000)), // 5%: fine
            (PathBuf::from("fat.png"), result(100_000, 60_000)), // 40%: flagged
            (PathBuf::from("huge.jpg"), result(900_000, 890_000)), // over limit
        ];
        assert_eq!(report(&results, &limits, OutputMode::Quiet), 2);
    }

    #[test]
    fn an_unimprovable_file_passes() {
        let limits = Limits {
            max_size: None,
            min_savings: 10.0,
        };
        // new >= old: optimising can't help, so there is nothing to fix.
        let results = vec![(PathBuf::from("done.png"), result(50_000, 50_000))];
        assert_eq!(report(&results, &limits, OutputMode::Quiet), 0);
    }

    #[test]
    fn excludes_by_directory_name() {
        let ex = vec!["node_modules".to_string()];
        assert!(excluded(Path::new("web/node_modules/pkg/logo.png"), &ex));
        assert!(!excluded(Path::new("web/assets/logo.png"), &ex));
    }
}
