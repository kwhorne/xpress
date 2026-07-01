//! Live progress for batch operations.
//!
//! Runs jobs in parallel and, when stderr is a terminal and we're in normal
//! output mode, shows a spinner with a `[done/total]` counter and elapsed time.
//! Quiet/JSON modes and non-TTY stderr show nothing.

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rayon::prelude::*;

use xpress_core::result::{OptimisationResult, OptimiseError, OptimiseOptions};

use crate::render::OutputMode;

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Run `f` over every job in parallel, returning per-job results. Renders a live
/// progress line while work is in flight (best-effort, terminal only).
pub fn run_jobs<F>(
    jobs: Vec<(PathBuf, OptimiseOptions)>,
    mode: OutputMode,
    max_jobs: Option<usize>,
    f: F,
) -> Vec<(PathBuf, Result<OptimisationResult, OptimiseError>)>
where
    F: Fn(&Path, &OptimiseOptions) -> Result<OptimisationResult, OptimiseError> + Sync + Send,
{
    let total = jobs.len();
    let show = mode == OutputMode::Normal && std::io::stderr().is_terminal() && total > 0;

    let done = Arc::new(AtomicUsize::new(0));
    let stop = Arc::new(AtomicBool::new(false));

    let ticker = if show {
        let done = done.clone();
        let stop = stop.clone();
        Some(std::thread::spawn(move || {
            let start = Instant::now();
            let mut frame = 0usize;
            while !stop.load(Ordering::Relaxed) {
                let d = done.load(Ordering::Relaxed);
                let secs = start.elapsed().as_secs();
                eprint!(
                    "\r\x1b[2K{} [{d}/{total}] · {secs}s",
                    SPINNER[frame % SPINNER.len()]
                );
                let _ = std::io::stderr().flush();
                frame += 1;
                std::thread::sleep(Duration::from_millis(100));
            }
        }))
    } else {
        None
    };

    let run = || -> Vec<_> {
        jobs.par_iter()
            .map(|(p, o)| {
                let r = f(p, o);
                done.fetch_add(1, Ordering::Relaxed);
                (p.clone(), r)
            })
            .collect()
    };
    // Cap parallelism with a scoped pool when requested; otherwise use the global pool.
    let results: Vec<_> = match max_jobs.filter(|&n| n > 0) {
        Some(n) => rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build()
            .map(|pool| pool.install(run))
            .unwrap_or_else(|_| run()),
        None => run(),
    };

    if let Some(t) = ticker {
        stop.store(true, Ordering::Relaxed);
        let _ = t.join();
        eprint!("\r\x1b[2K"); // clear the progress line
        let _ = std::io::stderr().flush();
    }

    // Preserve input order for stable output.
    let mut ordered = results;
    ordered.sort_by_key(|(p, _)| {
        jobs.iter()
            .position(|(jp, _)| jp == p)
            .unwrap_or(usize::MAX)
    });
    ordered
}
