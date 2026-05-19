//! `app-macos` — desktop demo for `svg3` (macOS first).
//!
//! Status: scaffolding. This is a stub entry point: it initialises logging and
//! exercises the (not-yet-implemented) `svg3` pipeline, then exits cleanly. A
//! `winit` event loop and a `wgpu` surface are the next milestone.

use svg3::render::RenderConfig;

const DEMO: &str = r#"<scene><cube/></scene>"#;

fn main() {
    env_logger::init();

    log::info!("svg3 macOS demo — scaffold build");
    match svg3::render_str(DEMO, RenderConfig::default()) {
        Ok(()) => log::info!("pipeline completed"),
        Err(err) => log::info!("pipeline is still a scaffold: {err}"),
    }
}
