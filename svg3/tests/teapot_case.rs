//! SVG fixture for the 3D teapot snapshot test.
//!
//! Like the [`crab_cases`] fixtures, the teapot is built purely from svg3's
//! SPEC §5 3D primitives and shaded entirely by an SVG filter chain — no
//! scene-lighting feature is involved. It is the regression evidence for two
//! techniques the renderer relies on:
//!
//!   * **Filter shading of 3D primitives.** The `<ellipsoid>` body/lid/knob are
//!     flat-filled solids domed into ceramic by the shared `gloss` lighting
//!     filter (`feGaussianBlur` on `SourceAlpha` → `feDiffuseLighting` +
//!     `feSpecularLighting`). A bare 3D primitive renders flat; the filter is
//!     what makes it read as round.
//!   * **`<surface>` tubes from one canonical cross-section.** The spout and
//!     handle loft a single reused circle `d` whose per-ring `transform` varies
//!     (scale/rotate/translate). Because each child path flattens in its local
//!     frame *before* its transform, every ring keeps the same sample count, so
//!     the surface is well-formed.
//!
//! The document is the same file the `svg3-render` skill ships as a worked
//! example, pulled in with `include_str!` so the snapshot golden also guards
//! that shipped example against regressions (single source of truth — edit the
//! example, regenerate the golden).

#![allow(dead_code)]

/// Square render size for [`TEAPOT_SVG`] — the document is authored for a
/// 512×512 frame (1 user unit = 1 pixel; the root `viewBox` is ignored).
pub const TEAPOT_SIZE: u32 = 512;

/// The 3D teapot fixture, loaded from the skill's worked example.
pub const TEAPOT_SVG: &str = include_str!("../../svg3-skill/examples/teapot.svg3");
