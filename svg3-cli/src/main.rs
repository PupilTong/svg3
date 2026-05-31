//! `svg3` — a headless command-line renderer for svg3 / SVG documents.
//!
//! Reads an svg3/SVG document (from a file or stdin), rasterises it on the GPU,
//! and writes the result as a PNG. This is a thin wrapper over the [`svg3`]
//! crate's headless render API; `svg3/examples/shade_probe.rs` shows the
//! equivalent library-level flow.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::Parser;
use svg3::dom::parse;
use svg3::render::{Camera, Image, RenderConfig, RenderError, Renderer};

/// Render an svg3 / SVG document to a PNG image (headless GPU).
///
/// svg3 3D elements (`<cube>`, `<ellipsoid>`, `<cylinder>`, `<surface>`) are
/// only drawn when the root `<svg>` opts into 3D with `extension="pupiltong"`;
/// otherwise the document renders as plain SVG.
#[derive(Parser, Debug)]
#[command(name = "svg3", version, about)]
struct Args {
    /// Input svg3/SVG file. Use `-` to read the document from stdin.
    #[arg(value_name = "INPUT")]
    input: PathBuf,

    /// Output PNG path. Defaults to the input file with a `.png` extension;
    /// required when the document is read from stdin (`-`).
    #[arg(short = 'o', long, value_name = "FILE")]
    output: Option<PathBuf>,

    /// Output width in pixels.
    #[arg(short = 'W', long, default_value_t = 512)]
    width: u32,

    /// Output height in pixels.
    #[arg(short = 'H', long, default_value_t = 512)]
    height: u32,

    /// Frame 3D content with the perspective camera. Has no effect unless the
    /// root `<svg>` opts into 3D with `extension="pupiltong"`; a flat 2D
    /// document always renders head-on.
    #[arg(long)]
    camera: bool,

    /// Parse and report the document without rendering. Needs no GPU, so it
    /// also works on headless hosts — handy for validating input.
    #[arg(long)]
    check: bool,
}

fn main() {
    if let Err(err) = run() {
        // `{:#}` renders the full anyhow context chain on a single line.
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args = Args::parse();

    let source = read_input(&args.input)
        .with_context(|| format!("reading input from {}", display_input(&args.input)))?;

    let document = parse(&source).context("parsing svg3 document")?;

    if args.check {
        println!(
            "{}: parsed OK; would render at {}x{}",
            display_input(&args.input),
            args.width,
            args.height
        );
        return Ok(());
    }

    let renderer = match Renderer::headless() {
        Ok(renderer) => renderer,
        Err(RenderError::NoAdapter) => bail!(
            "no GPU adapter is available, so the document cannot be rendered. \
             Re-run with `--check` to validate the input without a GPU."
        ),
        Err(err) => return Err(err).context("initialising the headless renderer"),
    };

    let config = RenderConfig {
        width: args.width,
        height: args.height,
        camera: args.camera.then(|| Camera::facing(args.width, args.height)),
        ..RenderConfig::default()
    };

    let image = renderer
        .render_to_image(&document, config)
        .context("rendering the document")?;

    let output = resolve_output(&args)?;
    write_png(&output, &image).with_context(|| format!("writing PNG to {}", output.display()))?;

    eprintln!(
        "wrote {} ({}x{})",
        output.display(),
        image.width,
        image.height
    );
    Ok(())
}

/// Read the document source from a file, or from stdin when `input` is `-`.
fn read_input(input: &Path) -> Result<String> {
    if is_stdin(input) {
        Ok(std::io::read_to_string(std::io::stdin())?)
    } else {
        Ok(std::fs::read_to_string(input)?)
    }
}

/// Decide where the PNG should be written.
///
/// An explicit `-o` always wins. Otherwise the output is the input path with a
/// `.png` extension — except for stdin input, which has no path to derive from
/// and therefore requires `-o`.
fn resolve_output(args: &Args) -> Result<PathBuf> {
    if let Some(output) = &args.output {
        return Ok(output.clone());
    }
    if is_stdin(&args.input) {
        bail!("`--output`/`-o` is required when the document is read from stdin");
    }
    Ok(args.input.with_extension("png"))
}

/// Encode an [`Image`] (row-major RGBA8) as a PNG file, creating parent
/// directories as needed.
fn write_png(path: &Path, image: &Image) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating output directory {}", parent.display()))?;
        }
    }
    let file = std::fs::File::create(path)?;
    let mut encoder = png::Encoder::new(file, image.width, image.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().context("writing PNG header")?;
    writer
        .write_image_data(&image.pixels)
        .context("writing PNG data")?;
    Ok(())
}

/// Whether `input` denotes stdin (the conventional `-`).
fn is_stdin(input: &Path) -> bool {
    input.as_os_str() == "-"
}

/// A human-readable label for the input source, used in messages.
fn display_input(input: &Path) -> String {
    if is_stdin(input) {
        "<stdin>".to_string()
    } else {
        input.display().to_string()
    }
}
