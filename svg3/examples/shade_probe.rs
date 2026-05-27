//! Render the 3D-crab snapshot fixture to `target/shade-probe.png` for
//! fast visual iteration without going through the snapshot test runner.
//!
//! The SVG itself lives in `tests/crab_cases.rs` so the example and the
//! snapshot tests render the exact same document.
//!
//! ```sh
//! cargo run -p svg3 --example shade_probe
//! ```

use std::path::PathBuf;

use svg3::dom::parse;
use svg3::render::{RenderConfig, Renderer};

#[path = "../tests/crab_cases.rs"]
mod crab_cases;

fn main() {
    let renderer = Renderer::headless().expect("init headless renderer");
    let document = parse(crab_cases::CRAB_FULL_SVG).expect("parse crab SVG");

    let config = RenderConfig {
        width: crab_cases::CRAB_FULL_SIZE,
        height: crab_cases::CRAB_FULL_SIZE,
        ..RenderConfig::default()
    };

    let image = renderer
        .render_to_image(&document, config)
        .expect("render crab");

    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/shade-probe.png");
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).expect("create target dir");
    }
    let file = std::fs::File::create(&out).expect("create output PNG");
    let mut encoder = png::Encoder::new(file, image.width, image.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("write PNG header");
    writer
        .write_image_data(&image.pixels)
        .expect("write PNG data");

    eprintln!("wrote {}", out.display());
}
