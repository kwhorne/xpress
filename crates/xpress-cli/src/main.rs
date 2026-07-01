//! xpress — a media optimisation CLI.
//!
//! Subcommands:
//!   xpress optimise [image|video|pdf|audio|files] <items...>
//!   xpress downscale --factor <f> <items...>
//!   xpress convert --to <fmt> <items...>
//!   xpress strip-exif <items...>
//!   xpress doctor

mod progress;
mod render;
mod watch;

use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use clap::{Args, CommandFactory, Parser, Subcommand};

use xpress_core::audio::AudioFormat;
use xpress_core::collect_files;
use xpress_core::compression::{CompressionQuality, CompressionTier};
use xpress_core::filetype::MediaKind;
use xpress_core::result::OptimiseOptions;
use xpress_core::tools::{self, Tool};

const CREDITS: &str = "Developed by Knut W. Horne · https://kwhorne.com";
const LONG_VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    "\nDeveloped by Knut W. Horne · https://kwhorne.com"
);

#[derive(Parser)]
#[command(
    name = "xpress",
    version,
    long_version = LONG_VERSION,
    about = "Image, video, PDF and audio optimiser",
    after_help = CREDITS,
    after_long_help = CREDITS
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Optimise images, videos, audio files and PDFs.
    #[command(alias = "optimize")]
    Optimise(OptimiseArgs),
    /// Downscale and optimise images and videos by a factor (0.1–1.0).
    Downscale(DownscaleArgs),
    /// Convert images or audio files to another format.
    Convert(ConvertArgs),
    /// Crop and optimise images or videos to a size or aspect ratio.
    Crop(CropArgs),
    /// Run, save and manage pipelines (chains of steps).
    #[command(subcommand)]
    Pipeline(PipelineCmd),
    /// Watch folders (and/or the clipboard) and optimise automatically.
    Watch(WatchArgs),
    /// Crop PDFs to an aspect ratio non-destructively (sets the page CropBox).
    CropPdf(CropPdfArgs),
    /// Revert a non-destructive PDF crop (removes the CropBox).
    UncropPdf(FilesArg),
    /// Render PDF pages to images (png/jpeg) via ghostscript.
    ExtractPages(ExtractPagesArgs),
    /// Restore originals from `.orig` backups.
    Restore(FilesArg),
    /// Delete `.orig` backups.
    CleanBackups(FilesArg),
    /// Delete EXIF metadata from images.
    StripExif(FilesArg),
    /// Show the config file path and current default values.
    Config,
    /// Extract bundled binaries into the per-user bundle dir.
    Bundle,
    /// Check which external tools are available.
    Doctor,
    /// Print a shell completion script (bash, zsh, fish, powershell, elvish).
    Completions {
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
    /// Print a man page (roff) to stdout.
    Man,
    /// Check for a newer release and update the binary in place.
    Update {
        /// Only check and report; don't download or replace anything.
        #[arg(long)]
        check: bool,
    },
}

#[derive(Args)]
struct CommonOpts {
    /// Recurse into folders.
    #[arg(short, long)]
    recursive: bool,
    /// Compression amount: 5 (best quality) .. 100 (smallest). Default 30 (normal).
    #[arg(long)]
    compression: Option<i32>,
    /// Use the aggressive preset (factor 64).
    #[arg(short, long)]
    aggressive: bool,
    /// Strip non-essential metadata.
    #[arg(long)]
    strip_metadata: bool,
    /// Do not preserve original timestamps.
    #[arg(long)]
    no_preserve_dates: bool,
    /// Do not make a backup of the original.
    #[arg(long)]
    no_backup: bool,
    /// Write output even if it is larger than the original.
    #[arg(long)]
    allow_larger: bool,
    /// Output file (single input) or directory (multiple inputs).
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Output results as JSON (for scripting).
    #[arg(long)]
    json: bool,
    /// Only print errors and the final summary line.
    #[arg(short, long)]
    quiet: bool,
    /// Max files processed in parallel (default: number of CPUs).
    #[arg(short = 'j', long)]
    jobs: Option<usize>,
    /// Kill any single tool that runs longer than this many seconds (0 = no limit).
    #[arg(long)]
    timeout: Option<u64>,
}

impl CommonOpts {
    fn compression_quality(&self, cfg: &xpress_core::config::Config) -> CompressionQuality {
        if self.aggressive || cfg.aggressive {
            CompressionQuality::aggressive()
        } else if let Some(f) = self.compression {
            CompressionQuality::new(CompressionTier::Custom, f)
        } else {
            CompressionQuality::new(CompressionTier::Custom, cfg.compression)
        }
    }

    /// Apply the process-wide timeout from `--timeout`.
    fn apply_timeout(&self) {
        if let Some(secs) = self.timeout {
            let d = (secs > 0).then(|| std::time::Duration::from_secs(secs));
            xpress_core::tools::set_timeout(d);
        }
    }

    fn output_mode(&self) -> render::OutputMode {
        if self.json {
            render::OutputMode::Json
        } else if self.quiet {
            render::OutputMode::Quiet
        } else {
            render::OutputMode::Normal
        }
    }

    fn to_options(&self) -> OptimiseOptions {
        self.apply_timeout();
        // Flags override config; config overrides built-in defaults.
        let cfg = xpress_core::config::Config::load();
        OptimiseOptions {
            compression: self.compression_quality(&cfg),
            backup: !self.no_backup && cfg.backup,
            strip_metadata: self.strip_metadata || cfg.strip_metadata,
            preserve_dates: !self.no_preserve_dates && cfg.preserve_dates,
            output: self.output.clone(),
            allow_larger: self.allow_larger,
        }
    }
}

#[derive(Args)]
struct OptimiseArgs {
    #[command(flatten)]
    common: CommonOpts,
    /// Restrict to a media kind: image | video | pdf | audio.
    #[arg(long, value_parser = parse_kind)]
    kind: Option<MediaKind>,
    /// PDF render DPI for downsampling (48–300). Omit for no downsample.
    #[arg(long)]
    pdf_dpi: Option<i32>,
    /// Compress to fit a budget, e.g. 500kb, 1.5mb, 250000.
    #[arg(long, value_parser = parse_size)]
    max_size: Option<u64>,
    /// For images: try multiple formats and keep the smallest.
    #[arg(long)]
    adaptive: bool,
    /// Files, folders or globs to optimise.
    #[arg(required = true)]
    items: Vec<PathBuf>,
}

#[derive(Args)]
struct DownscaleArgs {
    #[command(flatten)]
    common: CommonOpts,
    /// Scale factor 0.1–1.0 (e.g. 0.5 = half resolution).
    #[arg(short, long, default_value_t = 0.5)]
    factor: f64,
    #[arg(required = true)]
    items: Vec<PathBuf>,
}

#[derive(Args)]
struct ConvertArgs {
    #[command(flatten)]
    common: CommonOpts,
    /// Target format. Image: webp|avif|heic|jxl|png|jpeg. Audio: aac|mp3|opus|wav|flac|aiff.
    #[arg(short, long)]
    to: String,
    /// Explicit audio bitrate in kbps.
    #[arg(long)]
    bitrate: Option<i32>,
    /// Use a hardware encoder (VideoToolbox) for video codecs on Apple Silicon.
    #[arg(long)]
    hw: bool,
    #[arg(required = true)]
    items: Vec<PathBuf>,
}

#[derive(Args)]
struct CropArgs {
    #[command(flatten)]
    common: CommonOpts,
    /// Target size `1200x630`, `1200x0`, `0x720`, aspect ratio `16:9`, or a single number.
    #[arg(short, long)]
    size: String,
    /// Treat a single-number size as the longer edge (keeps aspect, no crop).
    #[arg(short = 'l', long)]
    long_edge: bool,
    /// Crop by centring on detected features (needs vips).
    #[arg(long)]
    smart_crop: bool,
    #[arg(required = true)]
    items: Vec<PathBuf>,
}

#[derive(Subcommand)]
enum PipelineCmd {
    /// Run a pipeline (a saved name or inline DSL) on files.
    Run(PipelineRunArgs),
    /// Save a pipeline to the library by name.
    Add { name: String, dsl: String },
    /// List saved pipelines and folder automations.
    List,
    /// Show the steps of a saved pipeline.
    Show { name: String },
    /// Delete a saved pipeline.
    Delete { name: String },
    /// Attach a pipeline to a folder (or "clipboard") for automatic runs.
    Attach {
        /// Folder path, or the literal "clipboard".
        source: String,
        /// Saved pipeline name or inline DSL.
        pipeline: String,
        /// File type: all | image | video | audio | pdf.
        #[arg(long, default_value = "all")]
        r#type: String,
    },
    /// Remove an attachment for a source.
    Detach {
        /// Folder path, or the literal "clipboard".
        source: String,
    },
}

#[derive(Args)]
struct WatchArgs {
    #[command(flatten)]
    common: CommonOpts,
    /// Also watch the clipboard for copied images.
    #[arg(long)]
    clipboard: bool,
    /// Pipeline (saved name or inline DSL) to run on watched folders. Default: optimise.
    #[arg(short, long)]
    pipeline: Option<String>,
    /// Folders to watch. When omitted, the saved automations are used.
    folders: Vec<PathBuf>,
}

#[derive(Args)]
struct PipelineRunArgs {
    #[command(flatten)]
    common: CommonOpts,
    /// A saved pipeline name or an inline DSL string, e.g. 'crop(width: 1600) -> convert(to: webp)'.
    pipeline: String,
    #[arg(required = true)]
    items: Vec<PathBuf>,
}

#[derive(Args)]
struct FilesArg {
    #[arg(short, long)]
    recursive: bool,
    #[arg(required = true)]
    items: Vec<PathBuf>,
}

#[derive(Args)]
struct CropPdfArgs {
    /// Aspect ratio, e.g. `16:9` or `1.91:1`.
    #[arg(long)]
    ratio: String,
    /// Write next to the original with this suffix instead of in place.
    #[arg(long, default_value = "")]
    suffix: String,
    #[arg(short, long)]
    recursive: bool,
    #[arg(required = true)]
    items: Vec<PathBuf>,
}

#[derive(Args)]
struct ExtractPagesArgs {
    /// Output image format: png | jpeg.
    #[arg(long, default_value = "png")]
    format: String,
    /// Render resolution in DPI.
    #[arg(long, default_value_t = 150)]
    dpi: i32,
    /// Output directory (default: alongside each PDF).
    #[arg(short, long)]
    out: Option<PathBuf>,
    #[arg(short, long)]
    recursive: bool,
    #[arg(required = true)]
    items: Vec<PathBuf>,
}

/// Parse a human size like `500kb`, `1.5mb`, `250000`.
fn parse_size(v: &str) -> Result<u64, String> {
    let s = v.trim().to_ascii_lowercase();
    let (num, mult) = if let Some(n) = s.strip_suffix("mb") {
        (n.trim(), 1_000_000.0)
    } else if let Some(n) = s.strip_suffix("kb") {
        (n.trim(), 1_000.0)
    } else if let Some(n) = s.strip_suffix('b') {
        (n.trim(), 1.0)
    } else {
        (s.as_str(), 1.0)
    };
    let value: f64 = num.parse().map_err(|_| format!("bad size '{v}'"))?;
    Ok((value * mult) as u64)
}

/// Resolve the per-file output path from `--output`, supporting filename
/// templates (containing `%`) and output directories. `counter` feeds `%i`.
fn resolve_output(
    output: &Option<PathBuf>,
    source: &Path,
    counter: &mut u64,
    single: bool,
) -> Option<PathBuf> {
    let out = output.as_ref()?;
    let s = out.to_string_lossy();
    if xpress_core::template::is_template(&s) {
        return Some(xpress_core::template::expand(&s, source, counter));
    }
    if out.is_dir() {
        return Some(out.join(xpress_core::result::file_name_lossy(source)));
    }
    if single {
        Some(out.clone())
    } else {
        // A concrete file path with multiple inputs makes no sense; fall back to in-place.
        None
    }
}

/// Build per-file options, resolving an output template/dir (sequential counter).
fn per_file_options(
    common: &CommonOpts,
    source: &Path,
    counter: &std::cell::Cell<u64>,
    single: bool,
) -> OptimiseOptions {
    let mut o = common.to_options();
    let mut c = counter.get();
    o.output = resolve_output(&common.output, source, &mut c, single);
    counter.set(c);
    o
}

fn parse_kind(s: &str) -> Result<MediaKind, String> {
    match s.to_ascii_lowercase().as_str() {
        "image" | "images" => Ok(MediaKind::Image),
        "video" | "videos" => Ok(MediaKind::Video),
        "audio" => Ok(MediaKind::Audio),
        "pdf" => Ok(MediaKind::Pdf),
        other => Err(format!("unknown kind '{other}'")),
    }
}

/// Restore the default SIGPIPE handler so piping output to `head`/`less` and
/// closing early exits quietly instead of panicking on a broken pipe.
#[cfg(unix)]
fn reset_sigpipe() {
    // SAFETY: setting a signal disposition to the default is sound here.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

#[cfg(not(unix))]
fn reset_sigpipe() {}

fn main() {
    reset_sigpipe();
    if let Err(e) = run() {
        eprintln!("{} {e:#}", render::ERROR_X);
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    // Make embedded binaries available before resolving any tool.
    if let Err(e) = xpress_core::bundled::ensure_bundled() {
        eprintln!("{} could not extract bundled tools: {e}", render::WARN);
    }

    match cli.command {
        Command::Optimise(args) => run_optimise(args),
        Command::Downscale(args) => run_downscale(args),
        Command::Convert(args) => run_convert(args),
        Command::Crop(args) => run_crop(args),
        Command::Pipeline(cmd) => run_pipeline(cmd),
        Command::Watch(args) => {
            let options = args.common.to_options();
            watch::run(
                args.clipboard,
                args.common.recursive,
                args.folders,
                args.pipeline,
                options,
            )
        }
        Command::CropPdf(args) => run_crop_pdf(args),
        Command::UncropPdf(args) => run_uncrop_pdf(args),
        Command::ExtractPages(args) => run_extract_pages(args),
        Command::Restore(args) => run_restore(args),
        Command::CleanBackups(args) => run_clean_backups(args),
        Command::StripExif(args) => run_strip_exif(args),
        Command::Config => {
            let cfg = xpress_core::config::Config::load();
            if let Some(p) = xpress_core::config::Config::path() {
                println!("Config file: {}", p.display());
                if !p.exists() {
                    println!("(not created yet — using built-in defaults)");
                }
            }
            println!("\nDefaults:");
            println!("  compression   = {}", cfg.compression);
            println!("  aggressive    = {}", cfg.aggressive);
            println!("  backup        = {}", cfg.backup);
            println!("  strip_metadata= {}", cfg.strip_metadata);
            println!("  preserve_dates= {}", cfg.preserve_dates);
            Ok(())
        }
        Command::Bundle => run_bundle(),
        Command::Doctor => {
            render::doctor();
            Ok(())
        }
        Command::Completions { shell } => {
            let mut cmd = Cli::command();
            let name = cmd.get_name().to_string();
            clap_complete::generate(shell, &mut cmd, name, &mut std::io::stdout());
            Ok(())
        }
        Command::Man => {
            let man = clap_mangen::Man::new(Cli::command());
            man.render(&mut std::io::stdout())?;
            Ok(())
        }
        Command::Update { check } => run_update(check),
    }
}

fn run_update(check_only: bool) -> Result<()> {
    let current = env!("CARGO_PKG_VERSION");
    let info = xpress_core::update::check(current)
        .map_err(|e| anyhow::anyhow!("could not check for updates: {e}"))?;

    if !info.newer {
        println!("{} xpress is up to date (v{current})", render::CHECK);
        return Ok(());
    }

    println!(
        "{} update available: v{} {} v{}",
        render::CHECK,
        info.current,
        render::ARROW,
        info.latest
    );
    println!("    {}", info.url);

    if check_only {
        println!("    run `xpress update` to install it");
        return Ok(());
    }

    // Download the matching release asset and replace this binary in place.
    let result = self_update::backends::github::Update::configure()
        .repo_owner("kwhorne")
        .repo_name("xpress")
        .bin_name("xpress")
        .current_version(current)
        .bin_path_in_archive("xpress-v{{ version }}-{{ target }}/{{ bin }}")
        .show_download_progress(true)
        .no_confirm(false)
        .build()
        .and_then(|u| u.update());

    match result {
        Ok(status) => {
            println!("{} updated to v{}", render::CHECK, status.version());
            Ok(())
        }
        Err(e) => {
            eprintln!("{} automatic update failed: {e}", render::WARN);
            eprintln!("    download it manually from {}", info.url);
            Ok(())
        }
    }
}

fn run_bundle() -> Result<()> {
    let dir = xpress_core::tools::bundle_dir()
        .ok_or_else(|| anyhow::anyhow!("could not determine the per-user bundle directory"))?;
    let n = xpress_core::bundled::ensure_bundled()?;
    if n == 0 {
        println!(
            "{} no embedded binaries (build with --features embed-tools, or run scripts/fetch-tools.sh).\n    Bundle dir: {}",
            render::WARN,
            dir.display()
        );
    } else {
        println!(
            "{} extracted {n} binar{} to {}",
            render::CHECK,
            if n == 1 { "y" } else { "ies" },
            dir.display()
        );
    }
    Ok(())
}

fn run_optimise(args: OptimiseArgs) -> Result<()> {
    let kinds: Vec<MediaKind> = args.kind.into_iter().collect();
    let files = collect_files(&args.items, args.common.recursive, &kinds);
    if files.is_empty() {
        bail!("no optimisable files found");
    }
    let mode = args.common.output_mode();
    let single = files.len() == 1;
    let counter = std::cell::Cell::new(1u64);
    let jobs: Vec<(PathBuf, OptimiseOptions)> = files
        .iter()
        .map(|f| {
            (
                f.clone(),
                per_file_options(&args.common, f, &counter, single),
            )
        })
        .collect();

    let max_size = args.max_size;
    let adaptive = args.adaptive;
    let pdf_dpi = args.pdf_dpi;
    let results = progress::run_jobs(jobs, mode, args.common.jobs, |f, o| {
        if let Some(max) = max_size {
            xpress_core::budget::optimise_to_budget(f, max, o)
        } else if adaptive && xpress_core::filetype::classify(f) == Some(MediaKind::Image) {
            xpress_core::image::optimise_adaptive(f, o)
        } else {
            xpress_core::optimise_file(f, o, AudioFormat::SameAsInput, pdf_dpi)
        }
    });
    render::summarise(&results, mode);
    Ok(())
}

fn run_downscale(args: DownscaleArgs) -> Result<()> {
    if !(0.05..=1.0).contains(&args.factor) {
        bail!("--factor must be between 0.05 and 1.0");
    }
    // Only images and videos have a resolution axis.
    let files = collect_files(
        &args.items,
        args.common.recursive,
        &[MediaKind::Image, MediaKind::Video],
    );
    if files.is_empty() {
        bail!("no images or videos found");
    }
    let single = files.len() == 1;
    let counter = std::cell::Cell::new(1u64);
    let jobs: Vec<_> = files
        .iter()
        .map(|f| {
            (
                f.clone(),
                per_file_options(&args.common, f, &counter, single),
            )
        })
        .collect();
    let factor = args.factor;
    let results = progress::run_jobs(jobs, args.common.output_mode(), args.common.jobs, |f, o| {
        xpress_core::scale::downscale_file(f, factor, o)
    });
    render::summarise(&results, args.common.output_mode());
    Ok(())
}

fn run_convert(args: ConvertArgs) -> Result<()> {
    // Video -> GIF.
    if args.to.eq_ignore_ascii_case("gif") {
        let files = collect_files(&args.items, args.common.recursive, &[MediaKind::Video]);
        if files.is_empty() {
            bail!("no videos found to convert to GIF");
        }
        let single = files.len() == 1;
        let counter = std::cell::Cell::new(1u64);
        let jobs: Vec<_> = files
            .iter()
            .map(|f| {
                (
                    f.clone(),
                    per_file_options(&args.common, f, &counter, single),
                )
            })
            .collect();
        let results =
            progress::run_jobs(jobs, args.common.output_mode(), args.common.jobs, |f, o| {
                xpress_core::video::to_gif(f, o, 15, None)
            });
        render::summarise(&results, args.common.output_mode());
        return Ok(());
    }

    // Video codec conversion (mp4/hevc/av1/webm).
    if let Some(codec) = xpress_core::video::VideoCodec::from_target(&args.to) {
        let files = collect_files(&args.items, args.common.recursive, &[MediaKind::Video]);
        if !files.is_empty() {
            let single = files.len() == 1;
            let counter = std::cell::Cell::new(1u64);
            let jobs: Vec<_> = files
                .iter()
                .map(|f| {
                    (
                        f.clone(),
                        per_file_options(&args.common, f, &counter, single),
                    )
                })
                .collect();
            let hw = args.hw;
            let results =
                progress::run_jobs(jobs, args.common.output_mode(), args.common.jobs, |f, o| {
                    xpress_core::video::convert_codec(f, codec, o, hw)
                });
            render::summarise(&results, args.common.output_mode());
            return Ok(());
        }
        // No videos matched; fall through (e.g. `mp4` given but only images present).
    }

    // Dispatch on the target: image format vs audio format.
    if let Some(format) = xpress_core::image::ImageFormat::from_str(&args.to) {
        let files = collect_files(&args.items, args.common.recursive, &[MediaKind::Image]);
        if files.is_empty() {
            bail!("no images found");
        }
        let single = files.len() == 1;
        let counter = std::cell::Cell::new(1u64);
        let jobs: Vec<_> = files
            .iter()
            .map(|f| {
                (
                    f.clone(),
                    per_file_options(&args.common, f, &counter, single),
                )
            })
            .collect();
        let results =
            progress::run_jobs(jobs, args.common.output_mode(), args.common.jobs, |f, o| {
                xpress_core::image::convert(f, format, o)
            });
        render::summarise(&results, args.common.output_mode());
        return Ok(());
    }

    if let Some(format) = AudioFormat::from_target(&args.to) {
        let files = collect_files(&args.items, args.common.recursive, &[MediaKind::Audio]);
        if files.is_empty() {
            bail!("no audio files found");
        }
        let single = files.len() == 1;
        let counter = std::cell::Cell::new(1u64);
        let jobs: Vec<_> = files
            .iter()
            .map(|f| {
                (
                    f.clone(),
                    per_file_options(&args.common, f, &counter, single),
                )
            })
            .collect();
        let bitrate = args.bitrate;
        let results =
            progress::run_jobs(jobs, args.common.output_mode(), args.common.jobs, |f, o| {
                xpress_core::audio::optimise(f, o, format, bitrate)
            });
        render::summarise(&results, args.common.output_mode());
        return Ok(());
    }

    bail!(
        "unknown conversion target '{}': expected an image (webp, avif, heic, jxl, png, jpeg), audio (aac, mp3, opus, wav, flac, aiff) or video->gif format",
        args.to
    )
}

fn run_crop(args: CropArgs) -> Result<()> {
    let spec = xpress_core::crop::CropSpec::parse(&args.size)
        .map_err(|e| anyhow::anyhow!(e))?
        .with_long_edge(args.long_edge)
        .with_smart(args.smart_crop);
    let files = collect_files(
        &args.items,
        args.common.recursive,
        &[MediaKind::Image, MediaKind::Video],
    );
    if files.is_empty() {
        bail!("no images or videos found");
    }
    let single = files.len() == 1;
    let counter = std::cell::Cell::new(1u64);
    let jobs: Vec<_> = files
        .iter()
        .map(|f| {
            (
                f.clone(),
                per_file_options(&args.common, f, &counter, single),
            )
        })
        .collect();
    let results = progress::run_jobs(jobs, args.common.output_mode(), args.common.jobs, |f, o| {
        xpress_core::crop::crop_file(f, &spec, o)
    });
    render::summarise(&results, args.common.output_mode());
    Ok(())
}

fn run_pipeline(cmd: PipelineCmd) -> Result<()> {
    use xpress_core::store::Store;
    match cmd {
        PipelineCmd::Add { name, dsl } => {
            // Validate before saving.
            xpress_core::pipeline::parse(&dsl)
                .map_err(|e| anyhow::anyhow!("invalid pipeline: {e}"))?;
            let mut store = Store::load();
            store.pipelines.insert(name.clone(), dsl.clone());
            store.save()?;
            println!("{} saved pipeline '{name}': {dsl}", render::CHECK);
            Ok(())
        }
        PipelineCmd::List => {
            let store = Store::load();
            if store.pipelines.is_empty() {
                println!("No saved pipelines. Add one with: xpress pipeline add <name> '<dsl>'");
            } else {
                println!("Saved pipelines:");
                for (name, dsl) in &store.pipelines {
                    println!("  {name}: {dsl}");
                }
            }
            if !store.automations.is_empty() {
                println!("\nFolder automations:");
                for a in &store.automations {
                    println!("  {} [{}] -> {}", a.source, a.file_type, a.pipeline);
                }
            }
            if let Some(p) = Store::path() {
                println!("\n({})", p.display());
            }
            Ok(())
        }
        PipelineCmd::Show { name } => {
            let store = Store::load();
            let dsl = store
                .pipelines
                .get(&name)
                .ok_or_else(|| anyhow::anyhow!("no pipeline named '{name}'"))?;
            let steps = xpress_core::pipeline::parse(dsl).map_err(|e| anyhow::anyhow!(e))?;
            println!("{name}:");
            for (i, step) in steps.iter().enumerate() {
                println!("  {}. {step:?}", i + 1);
            }
            println!("\n  {}", xpress_core::pipeline::to_dsl(&steps));
            Ok(())
        }
        PipelineCmd::Delete { name } => {
            let mut store = Store::load();
            if store.pipelines.remove(&name).is_some() {
                store.save()?;
                println!("{} deleted pipeline '{name}'", render::CHECK);
            } else {
                bail!("no pipeline named '{name}'");
            }
            Ok(())
        }
        PipelineCmd::Attach {
            source,
            pipeline,
            r#type,
        } => {
            // Validate the pipeline (name or DSL) before saving.
            let dsl = {
                let s = Store::load();
                s.pipelines
                    .get(&pipeline)
                    .cloned()
                    .unwrap_or_else(|| pipeline.clone())
            };
            xpress_core::pipeline::parse(&dsl)
                .map_err(|e| anyhow::anyhow!("invalid pipeline: {e}"))?;
            let mut store = Store::load();
            store.automations.retain(|a| a.source != source);
            store.automations.push(xpress_core::store::Automation {
                source: source.clone(),
                file_type: r#type.clone(),
                pipeline: pipeline.clone(),
            });
            store.save()?;
            println!(
                "{} attached '{pipeline}' to {source} [{}]",
                render::CHECK,
                r#type
            );
            Ok(())
        }
        PipelineCmd::Detach { source } => {
            let mut store = Store::load();
            let before = store.automations.len();
            store.automations.retain(|a| a.source != source);
            if store.automations.len() == before {
                bail!("no attachment for source '{source}'");
            }
            store.save()?;
            println!("{} detached {source}", render::CHECK);
            Ok(())
        }
        PipelineCmd::Run(args) => run_pipeline_run(args),
    }
}

fn run_pipeline_run(args: PipelineRunArgs) -> Result<()> {
    // Resolve: a saved name, else treat the argument as inline DSL.
    let store = xpress_core::store::Store::load();
    let dsl = store
        .pipelines
        .get(&args.pipeline)
        .cloned()
        .unwrap_or_else(|| args.pipeline.clone());
    let steps =
        xpress_core::pipeline::parse(&dsl).map_err(|e| anyhow::anyhow!("invalid pipeline: {e}"))?;

    let files = collect_files(&args.items, args.common.recursive, &[]);
    if files.is_empty() {
        bail!("no files found");
    }
    let single = files.len() == 1;
    let counter = std::cell::Cell::new(1u64);
    let jobs: Vec<_> = files
        .iter()
        .map(|f| {
            (
                f.clone(),
                per_file_options(&args.common, f, &counter, single),
            )
        })
        .collect();
    let results = progress::run_jobs(jobs, args.common.output_mode(), args.common.jobs, |f, o| {
        xpress_core::pipeline::run(f, &steps, o)
    });
    render::summarise(&results, args.common.output_mode());
    Ok(())
}

fn parse_ratio(s: &str) -> Result<(f64, f64)> {
    let (a, b) = s
        .split_once(':')
        .ok_or_else(|| anyhow::anyhow!("ratio must look like 16:9"))?;
    Ok((a.trim().parse()?, b.trim().parse()?))
}

fn run_crop_pdf(args: CropPdfArgs) -> Result<()> {
    let aspect = parse_ratio(&args.ratio)?;
    let files = collect_files(&args.items, args.recursive, &[MediaKind::Pdf]);
    if files.is_empty() {
        bail!("no PDFs found");
    }
    let mut n = 0;
    for f in &files {
        let out = if args.suffix.is_empty() {
            f.clone()
        } else {
            let stem = f.file_stem().unwrap_or_default().to_string_lossy();
            f.with_file_name(format!("{stem}{}.pdf", args.suffix))
        };
        // Crop to a temp then move, so in-place crop is safe.
        let tmp = std::env::temp_dir().join(format!("xpress-crop-{}.pdf", std::process::id()));
        match xpress_core::pdf::crop(f, &tmp, aspect).and_then(|_| {
            std::fs::rename(&tmp, &out)
                .or_else(|_| std::fs::copy(&tmp, &out).map(|_| ()))
                .map_err(xpress_core::result::OptimiseError::Io)
        }) {
            Ok(()) => {
                n += 1;
                println!(
                    "{} cropped {} {} {}",
                    render::CHECK,
                    f.display(),
                    render::ARROW,
                    out.display()
                );
            }
            Err(e) => eprintln!("{} {} {} {e}", render::ERROR_X, f.display(), render::ARROW),
        }
    }
    println!("\n{n} cropped");
    Ok(())
}

fn run_uncrop_pdf(args: FilesArg) -> Result<()> {
    let files = collect_files(&args.items, args.recursive, &[MediaKind::Pdf]);
    if files.is_empty() {
        bail!("no PDFs found");
    }
    let mut n = 0;
    for f in &files {
        let tmp = std::env::temp_dir().join(format!("xpress-uncrop-{}.pdf", std::process::id()));
        match xpress_core::pdf::uncrop(f, &tmp).and_then(|_| {
            std::fs::rename(&tmp, f)
                .or_else(|_| std::fs::copy(&tmp, f).map(|_| ()))
                .map_err(xpress_core::result::OptimiseError::Io)
        }) {
            Ok(()) => {
                n += 1;
                println!("{} uncropped {}", render::CHECK, f.display());
            }
            Err(e) => eprintln!("{} {} {} {e}", render::ERROR_X, f.display(), render::ARROW),
        }
    }
    println!("\n{n} uncropped");
    Ok(())
}

fn run_extract_pages(args: ExtractPagesArgs) -> Result<()> {
    let files = collect_files(&args.items, args.recursive, &[MediaKind::Pdf]);
    if files.is_empty() {
        bail!("no PDFs found");
    }
    for f in &files {
        let out_dir = args
            .out
            .clone()
            .unwrap_or_else(|| f.parent().map(|p| p.to_path_buf()).unwrap_or_default());
        match xpress_core::pdf::extract_pages(f, &out_dir, &args.format, args.dpi) {
            Ok(pages) => println!(
                "{} {} {} {} page(s)",
                render::CHECK,
                f.display(),
                render::ARROW,
                pages.len()
            ),
            Err(e) => eprintln!("{} {} {} {e}", render::ERROR_X, f.display(), render::ARROW),
        }
    }
    Ok(())
}

fn run_restore(args: FilesArg) -> Result<()> {
    let backups = xpress_core::result::find_backups(&args.items, args.recursive);
    if backups.is_empty() {
        bail!("no .orig backups found");
    }
    let mut n = 0;
    for (backup, original) in &backups {
        match std::fs::rename(backup, original) {
            Ok(()) => {
                n += 1;
                println!("{} restored {}", render::CHECK, original.display());
            }
            Err(e) => eprintln!(
                "{} {} {} {e}",
                render::ERROR_X,
                original.display(),
                render::ARROW
            ),
        }
    }
    println!("\n{n} restored");
    Ok(())
}

fn run_clean_backups(args: FilesArg) -> Result<()> {
    let backups = xpress_core::result::find_backups(&args.items, args.recursive);
    if backups.is_empty() {
        bail!("no .orig backups found");
    }
    let mut n = 0;
    for (backup, _) in &backups {
        match std::fs::remove_file(backup) {
            Ok(()) => {
                n += 1;
                println!("{} removed {}", render::CHECK, backup.display());
            }
            Err(e) => eprintln!(
                "{} {} {} {e}",
                render::ERROR_X,
                backup.display(),
                render::ARROW
            ),
        }
    }
    println!("\n{n} backups deleted");
    Ok(())
}

fn run_strip_exif(args: FilesArg) -> Result<()> {
    if !tools::is_available(Tool::Exiftool) {
        bail!("exiftool not found — install it (e.g. `brew install exiftool`)");
    }
    let files = collect_files(&args.items, args.recursive, &[MediaKind::Image]);
    if files.is_empty() {
        bail!("no images found");
    }
    for f in &files {
        tools::copy_exif(f, f, true, &[]);
        println!("{} stripped metadata from {}", render::CHECK, f.display());
    }
    Ok(())
}
