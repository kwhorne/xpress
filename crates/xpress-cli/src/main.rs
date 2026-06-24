//! xpress — a media optimisation CLI.
//!
//! Subcommands:
//!   xpress optimise [image|video|pdf|audio|files] <items...>
//!   xpress downscale --factor <f> <items...>
//!   xpress convert --to <fmt> <items...>
//!   xpress strip-exif <items...>
//!   xpress doctor

mod render;
mod watch;

use std::path::PathBuf;

use anyhow::{bail, Result};
use clap::{Args, Parser, Subcommand};

use xpress_core::audio::AudioFormat;
use xpress_core::compression::{CompressionQuality, CompressionTier};
use xpress_core::filetype::MediaKind;
use xpress_core::result::OptimiseOptions;
use xpress_core::tools::{self, Tool};
use xpress_core::{collect_files, optimise_many};

#[derive(Parser)]
#[command(
    name = "xpress",
    version,
    about = "Image, video, PDF and audio optimiser"
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
    /// Delete EXIF metadata from images.
    StripExif(FilesArg),
    /// Extract bundled binaries into the per-user bundle dir.
    Bundle,
    /// Check which external tools are available.
    Doctor,
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
}

impl CommonOpts {
    fn compression_quality(&self) -> CompressionQuality {
        if self.aggressive {
            CompressionQuality::aggressive()
        } else if let Some(f) = self.compression {
            CompressionQuality::new(CompressionTier::Custom, f)
        } else {
            CompressionQuality::normal()
        }
    }

    fn to_options(&self) -> OptimiseOptions {
        OptimiseOptions {
            compression: self.compression_quality(),
            backup: !self.no_backup,
            strip_metadata: self.strip_metadata,
            preserve_dates: !self.no_preserve_dates,
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

fn parse_kind(s: &str) -> Result<MediaKind, String> {
    match s.to_ascii_lowercase().as_str() {
        "image" | "images" => Ok(MediaKind::Image),
        "video" | "videos" => Ok(MediaKind::Video),
        "audio" => Ok(MediaKind::Audio),
        "pdf" => Ok(MediaKind::Pdf),
        other => Err(format!("unknown kind '{other}'")),
    }
}

fn main() {
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
        Command::StripExif(args) => run_strip_exif(args),
        Command::Bundle => run_bundle(),
        Command::Doctor => {
            render::doctor();
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
    let options = args.common.to_options();
    let results = optimise_many(&files, &options, AudioFormat::SameAsInput, args.pdf_dpi);
    render::summarise(&results);
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
    let options = args.common.to_options();
    let single = files.len() == 1;
    let results: Vec<_> = files
        .iter()
        .map(|f| {
            let mut opts = options.clone();
            if !single {
                opts.output = None; // per-file output only makes sense for a single input
            }
            (
                f.clone(),
                xpress_core::scale::downscale_file(f, args.factor, &opts),
            )
        })
        .collect();
    render::summarise(&results);
    Ok(())
}

fn run_convert(args: ConvertArgs) -> Result<()> {
    let options = args.common.to_options();

    // Dispatch on the target: image format vs audio format.
    if let Some(format) = xpress_core::image::ImageFormat::from_str(&args.to) {
        let files = collect_files(&args.items, args.common.recursive, &[MediaKind::Image]);
        if files.is_empty() {
            bail!("no images found");
        }
        let single = files.len() == 1;
        let results: Vec<_> = files
            .iter()
            .map(|f| {
                let mut opts = options.clone();
                if !single {
                    opts.output = None;
                }
                (f.clone(), xpress_core::image::convert(f, format, &opts))
            })
            .collect();
        render::summarise(&results);
        return Ok(());
    }

    if let Some(format) = AudioFormat::from_target(&args.to) {
        let files = collect_files(&args.items, args.common.recursive, &[MediaKind::Audio]);
        if files.is_empty() {
            bail!("no audio files found");
        }
        let results: Vec<_> = files
            .iter()
            .map(|f| {
                (
                    f.clone(),
                    xpress_core::audio::optimise(f, &options, format, args.bitrate),
                )
            })
            .collect();
        render::summarise(&results);
        return Ok(());
    }

    bail!(
        "unknown conversion target '{}': expected an image (webp, avif, heic, jxl, png, jpeg) or audio (aac, mp3, opus, wav, flac, aiff) format",
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
    let options = args.common.to_options();
    let single = files.len() == 1;
    let results: Vec<_> = files
        .iter()
        .map(|f| {
            let mut opts = options.clone();
            if !single {
                opts.output = None;
            }
            (f.clone(), xpress_core::crop::crop_file(f, &spec, &opts))
        })
        .collect();
    render::summarise(&results);
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
    let options = args.common.to_options();
    let single = files.len() == 1;
    let results: Vec<_> = files
        .iter()
        .map(|f| {
            let mut opts = options.clone();
            if !single {
                opts.output = None;
            }
            (f.clone(), xpress_core::pipeline::run(f, &steps, &opts))
        })
        .collect();
    render::summarise(&results);
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
