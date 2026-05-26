//! SVG filter definition resolution for the renderer.
//!
//! Each `<filter>` definition is parsed into an ordered chain of
//! [`FilterPrimitive`]s, one per supported `<fe…>` child. Each primitive
//! carries its `in` / `in2` / `result` wiring as [`FilterInput`] /
//! [`Option<String>`] alongside the kind-specific parameters, so the renderer
//! can execute the chain as a real DAG: an earlier `result="foo"` is
//! addressable later via `in="foo"` or `in2="foo"`, and the special inputs
//! `SourceGraphic` and `SourceAlpha` always remain reachable.
//!
//! Only the structural shape — primitive selection, attribute parsing, chain
//! order — lives here. The actual GPU passes that execute each variant live
//! in [`crate::render::gpu::Renderer`], `filter.wgsl`, and `image.wgsl`. PNG
//! data-URL `<feImage>` sources are represented structurally here; decoding
//! / uploading stays lazy in the renderer.
//!
//! `<clipPath>` definition collection and reference resolution lives in the
//! [`clip`] submodule, because clip-path runs *before* the filter chain
//! (SVG 2 render order) and benefits from being a clearly separate concern.

mod clip;

pub(crate) use clip::ClipPathDefinitions;

use std::collections::BTreeMap;

use crate::dom::{Document, Element, ElementKind, NodeId};

/// One resolved filter primitive in the chain, including its input/output
/// wiring.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FilterPrimitive {
    /// `in` attribute. Selects this primitive's first input texture.
    pub(crate) input: FilterInput,
    /// `in2` attribute. Selects this primitive's second input texture,
    /// used by multi-input primitives (e.g. `feDisplacementMap`).
    pub(crate) input2: FilterInput,
    /// `result` attribute. Names this primitive's output for later `in`
    /// / `in2` references.
    pub(crate) result: Option<String>,
    /// Optional `x`/`y`/`width`/`height` defining the primitive's subregion
    /// (SVG 1.1 §15.5). When `Some`, the primitive's output is clipped to
    /// this rect — pixels outside are transparent black. Resolved in
    /// user-space lengths; the renderer converts to UV via the viewport.
    pub(crate) subregion: Option<PrimitiveSubregion>,
    /// Kind-specific parameters.
    pub(crate) kind: FilterPrimitiveKind,
}

/// A primitive's subregion in user-space lengths. Percentages resolve
/// against the SVG viewport (primitiveUnits="userSpaceOnUse").
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PrimitiveSubregion {
    pub(crate) x: Option<Length>,
    pub(crate) y: Option<Length>,
    pub(crate) width: Option<Length>,
    pub(crate) height: Option<Length>,
}

impl PrimitiveSubregion {
    /// Resolve to UV-space (0..1 over the render extent) for the renderer.
    /// `viewport` is the SVG document's user-space viewport; the resulting
    /// UV maps to the full filter-region texture (which spans the viewport).
    pub(crate) fn to_uv(self, viewport: Viewport) -> [f32; 4] {
        let x = self
            .x
            .map(|l| l.resolve(viewport.width) / viewport.width)
            .unwrap_or(0.0);
        let y = self
            .y
            .map(|l| l.resolve(viewport.height) / viewport.height)
            .unwrap_or(0.0);
        let w = self
            .width
            .map(|l| l.resolve(viewport.width) / viewport.width)
            .unwrap_or(1.0);
        let h = self
            .height
            .map(|l| l.resolve(viewport.height) / viewport.height)
            .unwrap_or(1.0);
        [x, y, w, h]
    }
}

/// The kind-specific data carried by a [`FilterPrimitive`].
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum FilterPrimitiveKind {
    /// Gaussian blur — two separable passes.
    GaussianBlur(GaussianBlur),
    /// Embedded image source.
    Image(FilterImage),
    /// 4×5 colour matrix or one of the named shortcuts.
    ColorMatrix(ColorMatrix),
    /// Procedural Perlin / fractal noise generator (no input).
    Turbulence(Turbulence),
    /// Specular lighting computed from the alpha-as-height field.
    SpecularLighting(Lighting),
    /// Diffuse lighting computed from the alpha-as-height field.
    DiffuseLighting(Lighting),
    /// Square-radius dilate / erode.
    Morphology(Morphology),
    /// Solid colour flood over the filter region (no input).
    Flood(Flood),
    /// Blurred drop shadow composited under the source.
    DropShadow(DropShadow),
    /// Pixel displacement using a second input as offset map.
    DisplacementMap(DisplacementMap),
    /// 3×3 convolution kernel.
    ConvolveMatrix(ConvolveMatrix),
    /// Per-channel transfer function (identity / table / discrete / linear / gamma).
    ComponentTransfer(ComponentTransfer),
    /// Translate the input by (dx, dy) in filter-pixel space.
    Offset(Offset),
    /// Source-over composite of an ordered list of named inputs.
    Merge(Merge),
    /// Blend two inputs by SVG blend mode.
    Blend(Blend),
    /// Composite two inputs by Porter-Duff operator or arithmetic mode.
    Composite(Composite),
    /// Tile an input across the filter region.
    Tile,
}

impl FilterPrimitive {
    /// Substitute any `currentColor` placeholders on this primitive with
    /// `current_color`. SVG 1.1 `currentColor` resolves to the filtered
    /// element's `color` property; svg3::style isn't online yet, so scene.rs
    /// derives `current_color` from the filtered element's `color`
    /// attribute (with a black fallback).
    pub(crate) fn substitute_current_color(&mut self, current_color: [f32; 4]) {
        match &mut self.kind {
            FilterPrimitiveKind::Flood(f) if f.color_uses_current => {
                // Preserve the resolved flood-opacity (set into the alpha
                // channel by the parser).
                f.color = [
                    current_color[0],
                    current_color[1],
                    current_color[2],
                    current_color[3] * f.color[3],
                ];
            }
            FilterPrimitiveKind::DropShadow(d) if d.color_uses_current => {
                d.color = [
                    current_color[0],
                    current_color[1],
                    current_color[2],
                    current_color[3] * d.color[3],
                ];
            }
            FilterPrimitiveKind::SpecularLighting(l) | FilterPrimitiveKind::DiffuseLighting(l)
                if l.lighting_color_uses_current =>
            {
                l.lighting_color = current_color;
            }
            _ => {}
        }
    }

    /// Whether this primitive changes its input. Used to skip no-op chains.
    pub(crate) fn is_visible(&self) -> bool {
        match &self.kind {
            FilterPrimitiveKind::GaussianBlur(blur) => blur.is_visible(),
            FilterPrimitiveKind::Image(image) => !image.href.trim().is_empty(),
            FilterPrimitiveKind::ColorMatrix(_) => true,
            FilterPrimitiveKind::Turbulence(_) => true,
            FilterPrimitiveKind::SpecularLighting(_) => true,
            FilterPrimitiveKind::DiffuseLighting(_) => true,
            FilterPrimitiveKind::Morphology(m) => m.radius_x > 0.0 || m.radius_y > 0.0,
            FilterPrimitiveKind::Flood(_) => true,
            FilterPrimitiveKind::DropShadow(_) => true,
            FilterPrimitiveKind::DisplacementMap(d) => d.scale != 0.0,
            FilterPrimitiveKind::ConvolveMatrix(_) => true,
            FilterPrimitiveKind::ComponentTransfer(_) => true,
            FilterPrimitiveKind::Offset(o) => o.dx != 0.0 || o.dy != 0.0,
            // Merge composites *inputs* into the output regardless of "is
            // any input non-default". A `<feMerge/>` with no `feMergeNode`
            // children defaults to a single source-graphic node per SVG.
            FilterPrimitiveKind::Merge(_) => true,
            FilterPrimitiveKind::Blend(_) => true,
            FilterPrimitiveKind::Composite(_) => true,
            FilterPrimitiveKind::Tile => true,
        }
    }
}

/// One of an SVG filter primitive's `in` / `in2` input references.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) enum FilterInput {
    /// Attribute absent — SVG default: the previous primitive's output
    /// inside a chain, or the filter's `SourceGraphic` for the first
    /// primitive.
    #[default]
    Default,
    /// The filter's `SourceGraphic`: the rasterised filtered subtree.
    SourceGraphic,
    /// The filter's `SourceAlpha`: the alpha channel of `SourceGraphic`
    /// with RGB cleared to zero.
    SourceAlpha,
    /// The filter's `BackgroundImage`: the destination surface contents
    /// captured before the filtered element paints. SVG 1.1 §15.5 ties
    /// availability to `enable-background`; without `enable-background="new"`
    /// on an ancestor the pseudo-input is `undefined`. svg3 currently maps
    /// it to `SourceGraphic` (a documented approximation), which still
    /// satisfies the "primitive runs without crashing" invariant.
    BackgroundImage,
    /// The filter's `BackgroundAlpha`: the alpha channel of `BackgroundImage`
    /// with RGB cleared to zero. Approximated as `SourceAlpha` for the
    /// same reason as `BackgroundImage`.
    BackgroundAlpha,
    /// The filter's `FillPaint`: a fullscreen flood of the filtered
    /// element's resolved `fill` paint. Currently approximated as
    /// `SourceGraphic` until filter-local paint capture lands.
    FillPaint,
    /// The filter's `StrokePaint`: a fullscreen flood of the filtered
    /// element's resolved `stroke` paint. Currently approximated as
    /// `SourceGraphic` until filter-local paint capture lands.
    StrokePaint,
    /// A named earlier primitive's `result`.
    Named(String),
}

impl FilterInput {
    /// Parse an `in` / `in2` attribute value. Recognised pseudo-inputs are
    /// preserved as their dedicated variants; unrecognised non-empty values
    /// become `Named`. An empty / missing attribute is `Default`.
    pub(crate) fn parse(value: Option<&str>) -> Self {
        let Some(value) = value else {
            return Self::Default;
        };
        let value = value.trim();
        if value.is_empty() {
            return Self::Default;
        }
        match value {
            "SourceGraphic" => Self::SourceGraphic,
            "SourceAlpha" => Self::SourceAlpha,
            "BackgroundImage" => Self::BackgroundImage,
            "BackgroundAlpha" => Self::BackgroundAlpha,
            "FillPaint" => Self::FillPaint,
            "StrokePaint" => Self::StrokePaint,
            other => Self::Named(other.to_owned()),
        }
    }
}

use crate::render::shapes::Length;
use crate::render::Viewport;

/// A resolved `<feGaussianBlur>` primitive.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct GaussianBlur {
    /// Horizontal standard deviation in filter/render pixels.
    pub(crate) std_deviation_x: f32,
    /// Vertical standard deviation in filter/render pixels.
    pub(crate) std_deviation_y: f32,
}

impl GaussianBlur {
    /// Whether this blur changes the source image.
    pub(crate) fn is_visible(self) -> bool {
        self.std_deviation_x > 0.0 || self.std_deviation_y > 0.0
    }
}

/// A resolved `<feImage>` primitive.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FilterImage {
    /// Embedded image reference. The renderer currently supports PNG data
    /// URLs, decoded and uploaded lazily when the referenced filter paints.
    pub(crate) href: String,
    /// Optional top-left x position in SVG user space.
    x: Option<Length>,
    /// Optional top-left y position in SVG user space.
    y: Option<Length>,
    /// Optional rendered width in SVG user space.
    width: Option<Length>,
    /// Optional rendered height in SVG user space.
    height: Option<Length>,
    /// Resolved `preserveAspectRatio` (SVG 1.1 §7.10). `None` means the SVG
    /// default `xMidYMid meet` — preserving aspect ratio centred.
    /// `Some(PreserveAspect::None)` is the explicit "stretch" value.
    preserve_aspect: PreserveAspect,
}

/// Parsed `preserveAspectRatio`. Maps to SVG 1.1 §7.10 alignment values plus
/// the meet/slice rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PreserveAspect {
    /// `none`: stretch the image to fully fill the rect.
    None,
    /// Preserve aspect with the given alignment and meet/slice rule.
    Aligned { align: AspectAlign, slice: bool },
}

impl PreserveAspect {
    /// SVG 1.1 §7.10 default: `xMidYMid meet`.
    pub(crate) const DEFAULT: Self = Self::Aligned {
        align: AspectAlign::XMidYMid,
        slice: false,
    };
}

/// SVG 1.1 §7.10 alignment positions for `preserveAspectRatio`. The variant
/// names mirror the SVG attribute spelling exactly (`xMinYMin`, `xMidYMid`,
/// …) so the parser → variant mapping is mechanical; the `enum_variant_names`
/// lint flags the shared `X` prefix here, but renaming would diverge the
/// variants from spec terminology, which is a worse readability trade.
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AspectAlign {
    XMinYMin,
    XMidYMin,
    XMaxYMin,
    XMinYMid,
    XMidYMid,
    XMaxYMid,
    XMinYMax,
    XMidYMax,
    XMaxYMax,
}

impl FilterImage {
    /// Resolve the image rectangle once the decoded image's intrinsic pixel
    /// size is known. Percentages use the same root viewport basis as the
    /// rest of the renderer's length handling.
    ///
    /// The returned rect honors `preserveAspectRatio`: a non-`none` setting
    /// returns the actual *image draw* rect inside the bounding rect, with
    /// `meet` scaling to fit and `slice` scaling to cover. For `slice` the
    /// caller is responsible for clipping; current callers paint into a
    /// transparent texture sized to the bounding rect, so an oversized
    /// `slice` rect renders correctly clipped by the texture.
    pub(crate) fn resolve_rect(
        &self,
        viewport: Viewport,
        intrinsic_width: u32,
        intrinsic_height: u32,
    ) -> Option<ImageRect> {
        let width = self
            .width
            .map(|length| length.resolve(viewport.width))
            .unwrap_or(intrinsic_width as f32);
        let height = self
            .height
            .map(|length| length.resolve(viewport.height))
            .unwrap_or(intrinsic_height as f32);
        if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
            return None;
        }

        let x = self
            .x
            .map(|length| length.resolve(viewport.width))
            .unwrap_or(0.0);
        let y = self
            .y
            .map(|length| length.resolve(viewport.height))
            .unwrap_or(0.0);
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        let bounding = ImageRect {
            x,
            y,
            width,
            height,
        };
        Some(apply_preserve_aspect(
            bounding,
            intrinsic_width,
            intrinsic_height,
            self.preserve_aspect,
        ))
    }
}

/// Inset/expand `bounding` to the image's actual draw rect under the given
/// `preserveAspectRatio` rule.
fn apply_preserve_aspect(
    bounding: ImageRect,
    intrinsic_width: u32,
    intrinsic_height: u32,
    preserve: PreserveAspect,
) -> ImageRect {
    let (align, slice) = match preserve {
        PreserveAspect::None => return bounding,
        PreserveAspect::Aligned { align, slice } => (align, slice),
    };
    if intrinsic_width == 0 || intrinsic_height == 0 {
        return bounding;
    }
    let intrinsic_aspect = intrinsic_width as f32 / intrinsic_height as f32;
    let bounding_aspect = bounding.width / bounding.height;
    // `meet` fits inside (scale = min); `slice` covers (scale = max).
    let scale_x = bounding.width / intrinsic_width as f32;
    let scale_y = bounding.height / intrinsic_height as f32;
    let scale = if slice {
        scale_x.max(scale_y)
    } else {
        scale_x.min(scale_y)
    };
    let _ = (intrinsic_aspect, bounding_aspect); // documentation only
    let draw_width = intrinsic_width as f32 * scale;
    let draw_height = intrinsic_height as f32 * scale;
    let (fx, fy) = align_fractions(align);
    let dx = (bounding.width - draw_width) * fx;
    let dy = (bounding.height - draw_height) * fy;
    ImageRect {
        x: bounding.x + dx,
        y: bounding.y + dy,
        width: draw_width,
        height: draw_height,
    }
}

/// Fractional alignment offsets within the bounding rect for each
/// `AspectAlign` value. `(fx, fy)` ∈ {0.0, 0.5, 1.0}².
fn align_fractions(align: AspectAlign) -> (f32, f32) {
    match align {
        AspectAlign::XMinYMin => (0.0, 0.0),
        AspectAlign::XMidYMin => (0.5, 0.0),
        AspectAlign::XMaxYMin => (1.0, 0.0),
        AspectAlign::XMinYMid => (0.0, 0.5),
        AspectAlign::XMidYMid => (0.5, 0.5),
        AspectAlign::XMaxYMid => (1.0, 0.5),
        AspectAlign::XMinYMax => (0.0, 1.0),
        AspectAlign::XMidYMax => (0.5, 1.0),
        AspectAlign::XMaxYMax => (1.0, 1.0),
    }
}

fn parse_preserve_aspect_ratio(value: &str) -> PreserveAspect {
    let mut tokens = value.split_ascii_whitespace();
    let Some(first) = tokens.next() else {
        return PreserveAspect::DEFAULT;
    };
    if first.eq_ignore_ascii_case("none") {
        return PreserveAspect::None;
    }
    let align = match first {
        "xMinYMin" => AspectAlign::XMinYMin,
        "xMidYMin" => AspectAlign::XMidYMin,
        "xMaxYMin" => AspectAlign::XMaxYMin,
        "xMinYMid" => AspectAlign::XMinYMid,
        "xMidYMid" => AspectAlign::XMidYMid,
        "xMaxYMid" => AspectAlign::XMaxYMid,
        "xMinYMax" => AspectAlign::XMinYMax,
        "xMidYMax" => AspectAlign::XMidYMax,
        "xMaxYMax" => AspectAlign::XMaxYMax,
        // Per SVG 1.1 §7.10 the keyword starting with "defer" (SVG 1.2 spec)
        // is ignored; an unrecognised keyword falls back to the default.
        _ => AspectAlign::XMidYMid,
    };
    let slice = tokens
        .next()
        .map(str::to_ascii_lowercase)
        .as_deref()
        .map(|v| v == "slice")
        .unwrap_or(false);
    PreserveAspect::Aligned { align, slice }
}

/// A resolved `<feImage>` draw rectangle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ImageRect {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) width: f32,
    pub(crate) height: f32,
}

/// Resolved `<feColorMatrix>` data.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ColorMatrix {
    /// 4×5 row-major matrix that multiplies `[R, G, B, A, 1]`.
    pub(crate) matrix: [[f32; 5]; 4],
}

/// Resolved `<feTurbulence>` data.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Turbulence {
    /// Base frequency along x and y, in filter pixels.
    pub(crate) base_frequency: [f32; 2],
    /// Number of octaves to accumulate.
    pub(crate) num_octaves: u32,
    /// Random seed.
    pub(crate) seed: f32,
    /// `true` for `type="fractalNoise"`, `false` for `type="turbulence"`.
    pub(crate) fractal_noise: bool,
}

/// Resolved `<feSpecularLighting>` / `<feDiffuseLighting>` data.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Lighting {
    /// Surface scale — the alpha-as-height multiplier.
    pub(crate) surface_scale: f32,
    /// `kd` (diffuse) or `ks` (specular).
    pub(crate) constant: f32,
    /// Phong exponent — only used by the specular variant.
    pub(crate) specular_exponent: f32,
    /// Light colour, linear RGBA.
    pub(crate) lighting_color: [f32; 4],
    /// True if `lighting_color` was authored as `currentColor` — scene.rs
    /// will substitute the filtered element's resolved `color` attribute
    /// before the primitive is executed.
    pub(crate) lighting_color_uses_current: bool,
    /// Light source vector / position.
    pub(crate) light: LightSource,
}

/// Resolved light source for the lighting primitives.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum LightSource {
    /// `<feDistantLight>` — a parallel directional light. The vector points
    /// from the surface toward the light (unit length).
    Distant([f32; 3]),
    /// `<fePointLight>` — a position in filter space.
    Point([f32; 3]),
    /// `<feSpotLight>` — a positional cone light. `position` is the light
    /// origin; `direction` is the unit vector from the light toward
    /// `pointsAt`; `cone_exponent` is the SVG `specularExponent` cone
    /// falloff; `cos_limit` is `cos(limitingConeAngle)` (or `-1.0` when
    /// no limit is set).
    Spot {
        position: [f32; 3],
        direction: [f32; 3],
        cone_exponent: f32,
        cos_limit: f32,
    },
}

/// Resolved `<feMorphology>` data.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Morphology {
    /// Operator: `false` for `erode`, `true` for `dilate`.
    pub(crate) dilate: bool,
    /// Horizontal radius in filter pixels.
    pub(crate) radius_x: f32,
    /// Vertical radius in filter pixels.
    pub(crate) radius_y: f32,
}

/// Resolved `<feFlood>` data.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Flood {
    /// Flood colour in linear RGBA.
    pub(crate) color: [f32; 4],
    /// True if `flood-color` was authored as `currentColor` — substituted at
    /// scene-resolve time.
    pub(crate) color_uses_current: bool,
}

/// Resolved `<feDropShadow>` data.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct DropShadow {
    /// Horizontal blur standard deviation, in filter pixels.
    pub(crate) std_deviation_x: f32,
    /// Vertical blur standard deviation, in filter pixels.
    pub(crate) std_deviation_y: f32,
    /// Shadow offset, in filter pixels.
    pub(crate) offset: [f32; 2],
    /// Shadow colour (multiplied by source alpha), linear RGBA.
    pub(crate) color: [f32; 4],
    /// True if `flood-color` (shadow colour) was authored as `currentColor`.
    pub(crate) color_uses_current: bool,
}

/// Resolved `<feDisplacementMap>` data.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct DisplacementMap {
    /// Displacement scale, in filter pixels.
    pub(crate) scale: f32,
    /// Channel selector (0=R, 1=G, 2=B, 3=A) along x.
    pub(crate) x_channel: u32,
    /// Channel selector (0=R, 1=G, 2=B, 3=A) along y.
    pub(crate) y_channel: u32,
}

/// Resolved `<feConvolveMatrix>` data.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ConvolveMatrix {
    /// 3×3 kernel coefficients (row-major).
    pub(crate) kernel: [[f32; 3]; 3],
    /// Per-pixel divisor.
    pub(crate) divisor: f32,
    /// Constant addend applied after the kernel sum.
    pub(crate) bias: f32,
    /// `true` to preserve the source alpha unmodified.
    pub(crate) preserve_alpha: bool,
    /// SVG 1.1 §15.10 `edgeMode`: how to sample taps that fall outside the
    /// source. See `EDGE_MODE_*`.
    pub(crate) edge_mode: u32,
}

/// `edgeMode="duplicate"`: SVG's default — clamp out-of-bounds taps to the
/// nearest source edge pixel.
pub(crate) const EDGE_MODE_DUPLICATE: u32 = 0;
/// `edgeMode="wrap"`: out-of-bounds taps wrap around the source.
pub(crate) const EDGE_MODE_WRAP: u32 = 1;
/// `edgeMode="none"`: out-of-bounds taps are transparent black.
pub(crate) const EDGE_MODE_NONE: u32 = 2;

/// Resolved `<feComponentTransfer>` data: one transfer function per channel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ComponentTransfer {
    /// R-channel function.
    pub(crate) r: TransferFunction,
    /// G-channel function.
    pub(crate) g: TransferFunction,
    /// B-channel function.
    pub(crate) b: TransferFunction,
    /// A-channel function.
    pub(crate) a: TransferFunction,
}

/// Maximum number of `tableValues` entries supported per channel. SVG places
/// no upper bound, but practical filters use 2–4 entries; eight is generous
/// without ballooning the uniform.
pub(crate) const TRANSFER_TABLE_MAX: usize = 8;

/// One per-channel transfer function for `<feComponentTransfer>`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TransferFunction {
    /// Function kind tag — see `FN_*` constants.
    pub(crate) kind: u32,
    /// Up to [`TRANSFER_TABLE_MAX`] floats. Layout depends on `kind`:
    ///
    /// - `FN_LINEAR`: `[slope, intercept, 0, 0, …]`.
    /// - `FN_GAMMA`:  `[amplitude, exponent, offset, 0, …]`.
    /// - `FN_TABLE` / `FN_DISCRETE`: the first `count` slots hold the parsed
    ///   `tableValues`; the remainder are zero-padded.
    pub(crate) table: [f32; TRANSFER_TABLE_MAX],
    /// Valid entry count for `FN_TABLE` / `FN_DISCRETE`. Zero for the
    /// scalar function kinds.
    pub(crate) count: u32,
}

impl TransferFunction {
    /// SVG-1.1 default: identity (`y = C`).
    pub(crate) const IDENTITY: Self = Self {
        kind: FN_IDENTITY,
        table: [0.0; TRANSFER_TABLE_MAX],
        count: 0,
    };
}

/// Identity transfer function: `y = C`.
pub(crate) const FN_IDENTITY: u32 = 0;
/// Table transfer function: piecewise-linear over `count` control points.
pub(crate) const FN_TABLE: u32 = 1;
/// Discrete transfer function: piecewise-constant over `count` buckets.
pub(crate) const FN_DISCRETE: u32 = 2;
/// Linear transfer function: `y = slope * C + intercept`.
pub(crate) const FN_LINEAR: u32 = 3;
/// Gamma transfer function: `y = amplitude * C^exponent + offset`.
pub(crate) const FN_GAMMA: u32 = 4;

/// Displacement channel: alpha — the SVG default.
pub(crate) const CH_A: u32 = 3;

/// Resolved `<feOffset>` data.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Offset {
    /// Horizontal translation, in filter pixels.
    pub(crate) dx: f32,
    /// Vertical translation, in filter pixels.
    pub(crate) dy: f32,
}

/// Resolved `<feMerge>` data: an ordered list of `<feMergeNode in=…>` inputs.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Merge {
    /// One input per `<feMergeNode>` child, in document (painter) order.
    /// SVG 1.1 §15.13: each node's input contributes to the merged result in
    /// source-over order. An empty list defaults to the primitive's `in`.
    pub(crate) nodes: Vec<FilterInput>,
}

/// Resolved `<feBlend>` data.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Blend {
    /// Blend mode selector. See `BLEND_MODE_*`.
    pub(crate) mode: u32,
}

/// SVG 1.1 §15.7 blend modes plus the SVG-2 additions used by WPT.
pub(crate) const BLEND_MODE_NORMAL: u32 = 0;
pub(crate) const BLEND_MODE_MULTIPLY: u32 = 1;
pub(crate) const BLEND_MODE_SCREEN: u32 = 2;
pub(crate) const BLEND_MODE_DARKEN: u32 = 3;
pub(crate) const BLEND_MODE_LIGHTEN: u32 = 4;

/// Resolved `<feComposite>` data.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Composite {
    /// Operator selector. See `COMPOSITE_OP_*`.
    pub(crate) op: u32,
    /// Arithmetic coefficients k1..k4. Ignored unless `op == ARITHMETIC`.
    pub(crate) k: [f32; 4],
}

pub(crate) const COMPOSITE_OP_OVER: u32 = 0;
pub(crate) const COMPOSITE_OP_IN: u32 = 1;
pub(crate) const COMPOSITE_OP_OUT: u32 = 2;
pub(crate) const COMPOSITE_OP_ATOP: u32 = 3;
pub(crate) const COMPOSITE_OP_XOR: u32 = 4;
pub(crate) const COMPOSITE_OP_ARITHMETIC: u32 = 5;

/// Filter definitions keyed by their XML `id` attribute.
#[derive(Debug, Default)]
pub(crate) struct FilterDefinitions {
    filters: BTreeMap<String, Vec<FilterPrimitive>>,
}

/// The outcome of resolving an element's `filter="…"` reference.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum FilterResolution<'a> {
    /// A non-empty, supported filter primitive chain.
    Chain(&'a [FilterPrimitive]),
    /// SVG 1.1 §15.4: a `filter="url(#id)"` whose target is missing, empty,
    /// or contains no supported primitives still applies a filter — it just
    /// produces transparent black. Distinct from "no filter" because the
    /// element's geometry must NOT pass through to the destination.
    EmptyTransparent,
}

impl<'a> FilterResolution<'a> {
    /// The filter's primitive chain. `EmptyTransparent` returns an empty
    /// slice — most callers care about the chain contents, and a "this is
    /// transparent black" filter has nothing to execute.
    #[cfg(test)]
    pub(crate) fn chain(&self) -> &'a [FilterPrimitive] {
        match self {
            FilterResolution::Chain(chain) => chain,
            FilterResolution::EmptyTransparent => &[],
        }
    }
}

impl FilterDefinitions {
    /// Collect supported filter definitions from the whole document.
    ///
    /// If duplicate `id` values appear, the first supported definition wins,
    /// matching SVG's first-element lookup behavior for fragment references.
    ///
    /// `<filter>` elements may carry `href` / `xlink:href` pointing at
    /// another filter definition; SVG 1.1 §15.4 says the referenced
    /// definition's primitive list is inherited when the current filter
    /// has no children of its own. Two filter elements can reference each
    /// other circularly — we cap the chain at a small depth so a malformed
    /// document still resolves rather than crashes.
    pub(crate) fn collect(document: &Document) -> Self {
        let mut definitions = Self::default();

        // First pass: record raw chains keyed by id, alongside the
        // href target (if any) so we can resolve inheritance in pass 2.
        // BTreeMap iteration is alphabetical, so the resolution order is
        // deterministic regardless of source order.
        let mut raw: BTreeMap<String, (Vec<FilterPrimitive>, Option<String>)> = BTreeMap::new();
        let mut stack = vec![document.root()];
        while let Some(id) = stack.pop() {
            let node = document.node(id);
            if node.element.kind == ElementKind::Filter {
                if let Some(filter_id) = node.element.attributes.get("id") {
                    let chain = collect_primitives(document, node.children.iter().copied());
                    let href = filter_href_target(&node.element);
                    raw.entry(filter_id.to_owned()).or_insert((chain, href));
                }
                continue;
            }
            stack.extend(node.children.iter().rev().copied());
        }

        // Second pass: for each filter with no primitives of its own, walk
        // the href chain (up to MAX_HREF_DEPTH steps) and adopt the first
        // referenced ancestor's primitive list. Records the *resolved* chain
        // — independent of the ancestor's later mutation, since `raw` is
        // immutable once we entered pass 2.
        for (id, _) in raw.iter() {
            let resolved = resolve_href_chain(id, &raw);
            definitions.filters.insert(id.clone(), resolved);
        }
        definitions
    }

    /// Resolve an element's `filter="url(#id)"` presentation attribute.
    ///
    /// - `None`: the element has no `filter` attribute, or it is `none`.
    /// - `Some(EmptyTransparent)`: a `filter="url(#…)"` reference whose
    ///   target is missing, empty, or contains zero supported primitives —
    ///   per SVG 1.1 §15.4 the element renders as transparent black.
    /// - `Some(Chain(chain))`: the supported primitive chain to apply.
    pub(crate) fn resolve(&self, element: &Element) -> Option<FilterResolution<'_>> {
        let id = element
            .attributes
            .get("filter")
            .and_then(|value| filter_reference_id(value))?;
        match self.filters.get(id) {
            Some(chain) if !chain.is_empty() => Some(FilterResolution::Chain(chain.as_slice())),
            // Filter definition present but empty / unsupported primitives
            // only, or the definition is missing entirely.
            _ => Some(FilterResolution::EmptyTransparent),
        }
    }
}

/// Max steps to follow a filter's `href` chain before giving up. SVG doesn't
/// specify a hard cap, but anything beyond a couple of hops is virtually
/// always a malformed cycle.
const MAX_HREF_DEPTH: usize = 8;

/// Extract the `href` (or legacy `xlink:href`) target of a `<filter>` element,
/// returning the bare fragment id (no `#`) when present.
fn filter_href_target(element: &Element) -> Option<String> {
    let raw = element
        .attributes
        .get("href")
        .or_else(|| element.attributes.get("xlink:href"))?;
    let trimmed = raw.trim();
    trimmed.strip_prefix('#').map(|s| s.trim().to_owned())
}

/// Resolve a `<filter id=…>`'s effective primitive chain. If the filter has
/// no children of its own, walk its `href` ancestry up to `MAX_HREF_DEPTH`
/// hops looking for one that does — matching SVG 1.1 §15.4. Returns an empty
/// `Vec` for a circular or unresolvable chain.
fn resolve_href_chain(
    id: &str,
    raw: &BTreeMap<String, (Vec<FilterPrimitive>, Option<String>)>,
) -> Vec<FilterPrimitive> {
    let mut current = id.to_owned();
    let mut visited = std::collections::BTreeSet::new();
    for _ in 0..MAX_HREF_DEPTH {
        if !visited.insert(current.clone()) {
            // Cycle detected — abort the walk and yield no primitives so
            // the caller treats it as `EmptyTransparent`.
            return Vec::new();
        }
        let Some((chain, href)) = raw.get(&current) else {
            return Vec::new();
        };
        if !chain.is_empty() {
            return chain.clone();
        }
        match href {
            Some(next) => current = next.clone(),
            None => return Vec::new(),
        }
    }
    Vec::new()
}

fn collect_primitives(
    document: &Document,
    children: impl Iterator<Item = NodeId>,
) -> Vec<FilterPrimitive> {
    let mut chain = Vec::new();
    for child_id in children {
        let node = document.node(child_id);
        if let Some(primitive) = resolve_primitive(document, node) {
            chain.push(primitive);
        }
    }
    chain
}

fn resolve_primitive(document: &Document, node: &crate::dom::Node) -> Option<FilterPrimitive> {
    let kind = match node.element.kind {
        ElementKind::FeGaussianBlur => FilterPrimitiveKind::GaussianBlur(
            parse_std_deviation(
                node.element
                    .attributes
                    .get("stdDeviation")
                    .map(String::as_str),
            )
            .unwrap_or(GaussianBlur {
                std_deviation_x: 0.0,
                std_deviation_y: 0.0,
            }),
        ),
        ElementKind::FeImage => FilterPrimitiveKind::Image(resolve_fe_image(&node.element)?),
        ElementKind::FeColorMatrix => {
            FilterPrimitiveKind::ColorMatrix(parse_color_matrix(&node.element))
        }
        ElementKind::FeTurbulence => {
            FilterPrimitiveKind::Turbulence(parse_turbulence(&node.element))
        }
        ElementKind::FeSpecularLighting => {
            FilterPrimitiveKind::SpecularLighting(parse_lighting(document, node, true))
        }
        ElementKind::FeDiffuseLighting => {
            FilterPrimitiveKind::DiffuseLighting(parse_lighting(document, node, false))
        }
        ElementKind::FeMorphology => {
            FilterPrimitiveKind::Morphology(parse_morphology(&node.element))
        }
        ElementKind::FeFlood => FilterPrimitiveKind::Flood(parse_flood(&node.element)),
        ElementKind::FeDropShadow => {
            FilterPrimitiveKind::DropShadow(parse_drop_shadow(&node.element))
        }
        ElementKind::FeDisplacementMap => {
            FilterPrimitiveKind::DisplacementMap(parse_displacement_map(&node.element))
        }
        ElementKind::FeConvolveMatrix => {
            FilterPrimitiveKind::ConvolveMatrix(parse_convolve_matrix(&node.element))
        }
        ElementKind::FeComponentTransfer => {
            FilterPrimitiveKind::ComponentTransfer(parse_component_transfer(document, node))
        }
        ElementKind::FeOffset => FilterPrimitiveKind::Offset(parse_offset(&node.element)),
        ElementKind::FeMerge => FilterPrimitiveKind::Merge(parse_merge(document, node)),
        ElementKind::FeBlend => FilterPrimitiveKind::Blend(parse_blend(&node.element)),
        ElementKind::FeComposite => FilterPrimitiveKind::Composite(parse_composite(&node.element)),
        ElementKind::FeTile => FilterPrimitiveKind::Tile,
        _ => return None,
    };
    Some(FilterPrimitive {
        input: FilterInput::parse(node.element.attributes.get("in").map(String::as_str)),
        input2: FilterInput::parse(node.element.attributes.get("in2").map(String::as_str)),
        result: node
            .element
            .attributes
            .get("result")
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty()),
        subregion: parse_subregion(&node.element),
        kind,
    })
}

/// Parse `x/y/width/height` from a filter primitive. Returns `None` if none
/// of the four attributes is present — that's the spec default ("primitive
/// subregion equals the filter region") and saves a per-primitive clip pass.
fn parse_subregion(element: &Element) -> Option<PrimitiveSubregion> {
    let x = parse_length_attr(element, "x");
    let y = parse_length_attr(element, "y");
    let width = parse_length_attr(element, "width");
    let height = parse_length_attr(element, "height");
    if x.is_none() && y.is_none() && width.is_none() && height.is_none() {
        return None;
    }
    Some(PrimitiveSubregion {
        x,
        y,
        width,
        height,
    })
}

fn resolve_fe_image(element: &Element) -> Option<FilterImage> {
    let href = element
        .attributes
        .get("href")
        .or_else(|| element.attributes.get("xlink:href"))?
        .to_owned();
    let preserve_aspect = element
        .attributes
        .get("preserveAspectRatio")
        .map(|value| parse_preserve_aspect_ratio(value))
        .unwrap_or(PreserveAspect::DEFAULT);
    Some(FilterImage {
        href,
        x: parse_length_attr(element, "x"),
        y: parse_length_attr(element, "y"),
        width: parse_length_attr(element, "width"),
        height: parse_length_attr(element, "height"),
        preserve_aspect,
    })
}

fn parse_length_attr(element: &Element, name: &str) -> Option<Length> {
    element
        .attributes
        .get(name)
        .and_then(|value| Length::parse(value))
}

fn filter_reference_id(value: &str) -> Option<&str> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("none") {
        return None;
    }
    let inner = value.strip_prefix("url(")?.strip_suffix(')')?.trim();
    let inner = inner
        .strip_prefix('"')
        .and_then(|quoted| quoted.strip_suffix('"'))
        .or_else(|| {
            inner
                .strip_prefix('\'')
                .and_then(|quoted| quoted.strip_suffix('\''))
        })
        .unwrap_or(inner)
        .trim();
    let id = inner.strip_prefix('#')?.trim();
    (!id.is_empty()).then_some(id)
}

fn parse_std_deviation(value: Option<&str>) -> Option<GaussianBlur> {
    let value = value?;
    let values: Vec<f32> = value
        .split(|c: char| c == ',' || c.is_ascii_whitespace())
        .filter(|part| !part.is_empty())
        .map(str::parse::<f32>)
        .collect::<Result<_, _>>()
        .ok()?;
    let (x, y) = match values.as_slice() {
        [one] => (*one, *one),
        [x, y, ..] => (*x, *y),
        [] => return None,
    };
    (x.is_finite() && y.is_finite() && x >= 0.0 && y >= 0.0).then_some(GaussianBlur {
        std_deviation_x: x,
        std_deviation_y: y,
    })
}

fn parse_numbers(value: &str) -> Vec<f32> {
    value
        .split(|c: char| c == ',' || c.is_ascii_whitespace())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse::<f32>().ok())
        .filter(|v| v.is_finite())
        .collect()
}

fn parse_color_matrix(element: &Element) -> ColorMatrix {
    let kind = element
        .attributes
        .get("type")
        .map(|s| s.trim().to_ascii_lowercase())
        .unwrap_or_else(|| "matrix".to_owned());
    let values = element
        .attributes
        .get("values")
        .map(|value| parse_numbers(value))
        .unwrap_or_default();

    let mut matrix = identity_color_matrix();
    match kind.as_str() {
        "matrix" if values.len() == 20 => {
            for row in 0..4 {
                for col in 0..5 {
                    matrix[row][col] = values[row * 5 + col];
                }
            }
        }
        "saturate" => {
            // SVG 1.1 §15.18: saturate is a linear color matrix scaled by `s`.
            let s = values.first().copied().unwrap_or(1.0);
            matrix[0] = [
                0.213 + 0.787 * s,
                0.715 - 0.715 * s,
                0.072 - 0.072 * s,
                0.0,
                0.0,
            ];
            matrix[1] = [
                0.213 - 0.213 * s,
                0.715 + 0.285 * s,
                0.072 - 0.072 * s,
                0.0,
                0.0,
            ];
            matrix[2] = [
                0.213 - 0.213 * s,
                0.715 - 0.715 * s,
                0.072 + 0.928 * s,
                0.0,
                0.0,
            ];
            matrix[3] = [0.0, 0.0, 0.0, 1.0, 0.0];
        }
        "huerotate" => {
            // SVG 1.1 §15.18: hue rotation matrix combining the constant /
            // cosine / sine bases. Angle is degrees.
            let degrees = values.first().copied().unwrap_or(0.0);
            let theta = degrees.to_radians();
            let cos = theta.cos();
            let sin = theta.sin();
            matrix[0] = [
                0.213 + cos * 0.787 - sin * 0.213,
                0.715 - cos * 0.715 - sin * 0.715,
                0.072 - cos * 0.072 + sin * 0.928,
                0.0,
                0.0,
            ];
            matrix[1] = [
                0.213 - cos * 0.213 + sin * 0.143,
                0.715 + cos * 0.285 + sin * 0.140,
                0.072 - cos * 0.072 - sin * 0.283,
                0.0,
                0.0,
            ];
            matrix[2] = [
                0.213 - cos * 0.213 - sin * 0.787,
                0.715 - cos * 0.715 + sin * 0.715,
                0.072 + cos * 0.928 + sin * 0.072,
                0.0,
                0.0,
            ];
            matrix[3] = [0.0, 0.0, 0.0, 1.0, 0.0];
        }
        "luminancetoalpha" => {
            // SVG 1.1 §15.18 luminanceToAlpha — emit ITU-R BT.601 luma into alpha.
            matrix[0] = [0.0; 5];
            matrix[1] = [0.0; 5];
            matrix[2] = [0.0; 5];
            matrix[3] = [0.2125, 0.7154, 0.0721, 0.0, 0.0];
        }
        _ => {}
    }
    ColorMatrix { matrix }
}

fn identity_color_matrix() -> [[f32; 5]; 4] {
    [
        [1.0, 0.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 1.0, 0.0],
    ]
}

fn parse_turbulence(element: &Element) -> Turbulence {
    let base_frequency = element
        .attributes
        .get("baseFrequency")
        .map(|value| {
            let numbers = parse_numbers(value);
            match numbers.as_slice() {
                [x] => [*x, *x],
                [x, y, ..] => [*x, *y],
                [] => [0.0, 0.0],
            }
        })
        .unwrap_or([0.0, 0.0]);
    let num_octaves = element
        .attributes
        .get("numOctaves")
        .and_then(|s| s.trim().parse::<u32>().ok())
        .unwrap_or(1)
        .clamp(1, 8);
    let seed = element
        .attributes
        .get("seed")
        .and_then(|s| s.trim().parse::<f32>().ok())
        .unwrap_or(0.0);
    let fractal_noise = element
        .attributes
        .get("type")
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("fractalNoise"));
    Turbulence {
        base_frequency,
        num_octaves,
        seed,
        fractal_noise,
    }
}

fn parse_lighting(document: &Document, node: &crate::dom::Node, specular: bool) -> Lighting {
    let surface_scale = node
        .element
        .attributes
        .get("surfaceScale")
        .and_then(|s| s.trim().parse::<f32>().ok())
        .unwrap_or(1.0);
    let constant = node
        .element
        .attributes
        .get(if specular {
            "specularConstant"
        } else {
            "diffuseConstant"
        })
        .and_then(|s| s.trim().parse::<f32>().ok())
        .unwrap_or(1.0);
    let specular_exponent = node
        .element
        .attributes
        .get("specularExponent")
        .and_then(|s| s.trim().parse::<f32>().ok())
        .unwrap_or(1.0)
        .max(1.0);
    let raw_lighting = node.element.attributes.get("lighting-color");
    let lighting_color_uses_current = raw_lighting.is_some_and(|v| is_current_color(v));
    let lighting_color = raw_lighting
        .and_then(|s| crate::render::shapes::parse_color_value(s))
        .unwrap_or([1.0, 1.0, 1.0, 1.0]);
    let light = node
        .children
        .iter()
        .find_map(|child_id| parse_light_source(document.element(*child_id)))
        .unwrap_or(LightSource::Distant([0.0, 0.0, 1.0]));
    Lighting {
        surface_scale,
        constant,
        specular_exponent,
        lighting_color,
        lighting_color_uses_current,
        light,
    }
}

fn parse_light_source(element: &Element) -> Option<LightSource> {
    match element.kind {
        ElementKind::FeDistantLight => {
            let azimuth_deg = element
                .attributes
                .get("azimuth")
                .and_then(|s| s.trim().parse::<f32>().ok())
                .unwrap_or(0.0);
            let elevation_deg = element
                .attributes
                .get("elevation")
                .and_then(|s| s.trim().parse::<f32>().ok())
                .unwrap_or(0.0);
            let azimuth = azimuth_deg.to_radians();
            let elevation = elevation_deg.to_radians();
            // SVG 1.1 §15.21: the distant-light vector points from the
            // surface toward the light. In an SVG y-down frame, positive
            // elevation lifts the light up out of the page (negative y).
            let x = azimuth.cos() * elevation.cos();
            let y = -azimuth.sin() * elevation.cos();
            let z = elevation.sin();
            Some(LightSource::Distant([x, y, z]))
        }
        ElementKind::FePointLight => {
            let x = element
                .attributes
                .get("x")
                .and_then(|s| s.trim().parse::<f32>().ok())
                .unwrap_or(0.0);
            let y = element
                .attributes
                .get("y")
                .and_then(|s| s.trim().parse::<f32>().ok())
                .unwrap_or(0.0);
            let z = element
                .attributes
                .get("z")
                .and_then(|s| s.trim().parse::<f32>().ok())
                .unwrap_or(0.0);
            Some(LightSource::Point([x, y, z]))
        }
        ElementKind::FeSpotLight => {
            let read = |name: &str, default: f32| {
                element
                    .attributes
                    .get(name)
                    .and_then(|s| s.trim().parse::<f32>().ok())
                    .unwrap_or(default)
            };
            let position = [read("x", 0.0), read("y", 0.0), read("z", 0.0)];
            let aim = [
                read("pointsAtX", 0.0),
                read("pointsAtY", 0.0),
                read("pointsAtZ", 0.0),
            ];
            let mut dir = [
                aim[0] - position[0],
                aim[1] - position[1],
                aim[2] - position[2],
            ];
            let length = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
            if length > 1e-6 {
                dir[0] /= length;
                dir[1] /= length;
                dir[2] /= length;
            } else {
                // SVG-1.1 §15.21.1: when the light points at itself, fall
                // back to the +Z axis so the cone still has a well-defined
                // orientation.
                dir = [0.0, 0.0, 1.0];
            }
            let cone_exponent = element
                .attributes
                .get("specularExponent")
                .and_then(|s| s.trim().parse::<f32>().ok())
                .unwrap_or(1.0)
                .max(0.0);
            let cos_limit = element
                .attributes
                .get("limitingConeAngle")
                .and_then(|s| s.trim().parse::<f32>().ok())
                .map(|deg| deg.to_radians().cos())
                .unwrap_or(-1.0);
            Some(LightSource::Spot {
                position,
                direction: dir,
                cone_exponent,
                cos_limit,
            })
        }
        _ => None,
    }
}

fn parse_morphology(element: &Element) -> Morphology {
    let dilate = element
        .attributes
        .get("operator")
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("dilate"));
    let (radius_x, radius_y) = element
        .attributes
        .get("radius")
        .map(|value| {
            let numbers = parse_numbers(value);
            match numbers.as_slice() {
                [r] => (*r, *r),
                [x, y, ..] => (*x, *y),
                [] => (0.0, 0.0),
            }
        })
        .unwrap_or((0.0, 0.0));
    Morphology {
        dilate,
        radius_x: radius_x.max(0.0),
        radius_y: radius_y.max(0.0),
    }
}

fn parse_flood(element: &Element) -> Flood {
    let raw = element.attributes.get("flood-color");
    let color_uses_current = raw.is_some_and(|v| is_current_color(v));
    let mut color = raw
        .and_then(|value| crate::render::shapes::parse_color_value(value))
        .unwrap_or([0.0, 0.0, 0.0, 1.0]);
    if let Some(opacity) = element
        .attributes
        .get("flood-opacity")
        .and_then(|value| value.trim().parse::<f32>().ok())
    {
        color[3] *= opacity.clamp(0.0, 1.0);
    }
    Flood {
        color,
        color_uses_current,
    }
}

/// True if `value` is the case-insensitive `currentColor` keyword. SVG colour
/// values are otherwise case-sensitive, but the `currentColor` keyword is
/// historically permissive about case.
fn is_current_color(value: &str) -> bool {
    value.trim().eq_ignore_ascii_case("currentColor")
}

fn parse_drop_shadow(element: &Element) -> DropShadow {
    let blur = parse_std_deviation(element.attributes.get("stdDeviation").map(String::as_str))
        .unwrap_or(GaussianBlur {
            std_deviation_x: 2.0,
            std_deviation_y: 2.0,
        });
    let dx = element
        .attributes
        .get("dx")
        .and_then(|s| s.trim().parse::<f32>().ok())
        .unwrap_or(2.0);
    let dy = element
        .attributes
        .get("dy")
        .and_then(|s| s.trim().parse::<f32>().ok())
        .unwrap_or(2.0);
    let raw_color = element.attributes.get("flood-color");
    let color_uses_current = raw_color.is_some_and(|v| is_current_color(v));
    let mut color = raw_color
        .and_then(|value| crate::render::shapes::parse_color_value(value))
        .unwrap_or([0.0, 0.0, 0.0, 1.0]);
    if let Some(opacity) = element
        .attributes
        .get("flood-opacity")
        .and_then(|value| value.trim().parse::<f32>().ok())
    {
        color[3] *= opacity.clamp(0.0, 1.0);
    }
    DropShadow {
        std_deviation_x: blur.std_deviation_x,
        std_deviation_y: blur.std_deviation_y,
        offset: [dx, dy],
        color,
        color_uses_current,
    }
}

fn parse_displacement_map(element: &Element) -> DisplacementMap {
    let scale = element
        .attributes
        .get("scale")
        .and_then(|s| s.trim().parse::<f32>().ok())
        .unwrap_or(0.0);
    let x_channel = parse_channel(element.attributes.get("xChannelSelector"));
    let y_channel = parse_channel(element.attributes.get("yChannelSelector"));
    DisplacementMap {
        scale,
        x_channel,
        y_channel,
    }
}

fn parse_channel(value: Option<&String>) -> u32 {
    match value
        .map(|s| s.trim().to_ascii_uppercase())
        .as_deref()
        .unwrap_or("A")
    {
        "R" => 0,
        "G" => 1,
        "B" => 2,
        _ => CH_A,
    }
}

fn parse_convolve_matrix(element: &Element) -> ConvolveMatrix {
    let values = element
        .attributes
        .get("kernelMatrix")
        .map(|value| parse_numbers(value))
        .unwrap_or_default();
    let mut kernel = [[0.0_f32; 3]; 3];
    // SVG defaults the order to 3×3 when omitted. If the kernelMatrix has at
    // least 9 entries, fill the kernel row-major and ignore the rest; smaller
    // matrices fall back to the implicit identity.
    if values.len() >= 9 {
        for row in 0..3 {
            for col in 0..3 {
                kernel[row][col] = values[row * 3 + col];
            }
        }
    } else {
        kernel[1][1] = 1.0;
    }
    let sum: f32 = kernel.iter().flatten().sum();
    let divisor = element
        .attributes
        .get("divisor")
        .and_then(|s| s.trim().parse::<f32>().ok())
        .filter(|d| *d != 0.0)
        .unwrap_or(if sum == 0.0 { 1.0 } else { sum });
    let bias = element
        .attributes
        .get("bias")
        .and_then(|s| s.trim().parse::<f32>().ok())
        .unwrap_or(0.0);
    let preserve_alpha = element
        .attributes
        .get("preserveAlpha")
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("true"));
    let edge_mode = element
        .attributes
        .get("edgeMode")
        .map(|s| s.trim().to_ascii_lowercase())
        .as_deref()
        .map(parse_edge_mode)
        .unwrap_or(EDGE_MODE_DUPLICATE);
    ConvolveMatrix {
        kernel,
        divisor,
        bias,
        preserve_alpha,
        edge_mode,
    }
}

fn parse_edge_mode(value: &str) -> u32 {
    match value {
        "wrap" => EDGE_MODE_WRAP,
        "none" => EDGE_MODE_NONE,
        // SVG 1.1 §15.10: unrecognised values default to "duplicate".
        _ => EDGE_MODE_DUPLICATE,
    }
}

fn parse_component_transfer(document: &Document, node: &crate::dom::Node) -> ComponentTransfer {
    let mut transfer = ComponentTransfer {
        r: TransferFunction::IDENTITY,
        g: TransferFunction::IDENTITY,
        b: TransferFunction::IDENTITY,
        a: TransferFunction::IDENTITY,
    };
    for child_id in &node.children {
        let child = document.element(*child_id);
        let parsed = parse_transfer_function(child);
        match child.kind {
            ElementKind::FeFuncR => transfer.r = parsed,
            ElementKind::FeFuncG => transfer.g = parsed,
            ElementKind::FeFuncB => transfer.b = parsed,
            ElementKind::FeFuncA => transfer.a = parsed,
            _ => {}
        }
    }
    transfer
}

fn parse_transfer_function(element: &Element) -> TransferFunction {
    let kind = element
        .attributes
        .get("type")
        .map(|s| s.trim().to_ascii_lowercase())
        .unwrap_or_else(|| "identity".to_owned());
    match kind.as_str() {
        "identity" => TransferFunction::IDENTITY,
        "table" | "discrete" => {
            let kind_tag = if kind == "table" {
                FN_TABLE
            } else {
                FN_DISCRETE
            };
            let table_values = element
                .attributes
                .get("tableValues")
                .map(|value| parse_numbers(value))
                .unwrap_or_default();
            // SVG 1.1 §15.11: a table with fewer than two entries (or an
            // empty `tableValues`) is identity. Mirror that here so the
            // shader can treat any `count >= 2` as a valid table.
            if table_values.len() < 2 {
                return TransferFunction::IDENTITY;
            }
            let mut table = [0.0_f32; TRANSFER_TABLE_MAX];
            for (slot, value) in table.iter_mut().zip(table_values.iter()) {
                *slot = *value;
            }
            let count = table_values.len().min(TRANSFER_TABLE_MAX) as u32;
            TransferFunction {
                kind: kind_tag,
                table,
                count,
            }
        }
        "linear" => {
            let slope = element
                .attributes
                .get("slope")
                .and_then(|s| s.trim().parse::<f32>().ok())
                .unwrap_or(1.0);
            let intercept = element
                .attributes
                .get("intercept")
                .and_then(|s| s.trim().parse::<f32>().ok())
                .unwrap_or(0.0);
            let mut table = [0.0_f32; TRANSFER_TABLE_MAX];
            table[0] = slope;
            table[1] = intercept;
            TransferFunction {
                kind: FN_LINEAR,
                table,
                count: 0,
            }
        }
        "gamma" => {
            let amplitude = element
                .attributes
                .get("amplitude")
                .and_then(|s| s.trim().parse::<f32>().ok())
                .unwrap_or(1.0);
            let exponent = element
                .attributes
                .get("exponent")
                .and_then(|s| s.trim().parse::<f32>().ok())
                .unwrap_or(1.0);
            let offset = element
                .attributes
                .get("offset")
                .and_then(|s| s.trim().parse::<f32>().ok())
                .unwrap_or(0.0);
            let mut table = [0.0_f32; TRANSFER_TABLE_MAX];
            table[0] = amplitude;
            table[1] = exponent;
            table[2] = offset;
            TransferFunction {
                kind: FN_GAMMA,
                table,
                count: 0,
            }
        }
        _ => TransferFunction::IDENTITY,
    }
}

fn parse_offset(element: &Element) -> Offset {
    let dx = element
        .attributes
        .get("dx")
        .and_then(|s| s.trim().parse::<f32>().ok())
        .filter(|v| v.is_finite())
        .unwrap_or(0.0);
    let dy = element
        .attributes
        .get("dy")
        .and_then(|s| s.trim().parse::<f32>().ok())
        .filter(|v| v.is_finite())
        .unwrap_or(0.0);
    Offset { dx, dy }
}

fn parse_merge(document: &Document, node: &crate::dom::Node) -> Merge {
    let nodes = node
        .children
        .iter()
        .filter_map(|child_id| {
            let child = document.element(*child_id);
            (child.kind == ElementKind::FeMergeNode)
                .then(|| FilterInput::parse(child.attributes.get("in").map(String::as_str)))
        })
        .collect();
    Merge { nodes }
}

fn parse_blend(element: &Element) -> Blend {
    let mode = element
        .attributes
        .get("mode")
        .map(|s| s.trim().to_ascii_lowercase())
        .as_deref()
        .map(parse_blend_mode)
        .unwrap_or(BLEND_MODE_NORMAL);
    Blend { mode }
}

fn parse_blend_mode(value: &str) -> u32 {
    match value {
        "multiply" => BLEND_MODE_MULTIPLY,
        "screen" => BLEND_MODE_SCREEN,
        "darken" => BLEND_MODE_DARKEN,
        "lighten" => BLEND_MODE_LIGHTEN,
        // SVG 2 added more modes (overlay, color-dodge, …). Unrecognised
        // values fall back to `normal` so a document still renders.
        _ => BLEND_MODE_NORMAL,
    }
}

fn parse_composite(element: &Element) -> Composite {
    let op = element
        .attributes
        .get("operator")
        .map(|s| s.trim().to_ascii_lowercase())
        .as_deref()
        .map(parse_composite_op)
        .unwrap_or(COMPOSITE_OP_OVER);
    let k = [
        composite_k(element, "k1"),
        composite_k(element, "k2"),
        composite_k(element, "k3"),
        composite_k(element, "k4"),
    ];
    Composite { op, k }
}

fn composite_k(element: &Element, name: &str) -> f32 {
    element
        .attributes
        .get(name)
        .and_then(|s| s.trim().parse::<f32>().ok())
        .filter(|v| v.is_finite())
        .unwrap_or(0.0)
}

fn parse_composite_op(value: &str) -> u32 {
    match value {
        "in" => COMPOSITE_OP_IN,
        "out" => COMPOSITE_OP_OUT,
        "atop" => COMPOSITE_OP_ATOP,
        "xor" => COMPOSITE_OP_XOR,
        "arithmetic" => COMPOSITE_OP_ARITHMETIC,
        _ => COMPOSITE_OP_OVER,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_resolves_url_referenced_gaussian_blur() {
        let document = crate::dom::parse(
            r##"<svg><filter id="soft"><feGaussianBlur stdDeviation="4 2"/></filter><rect filter="url(#soft)"/></svg>"##,
        )
        .unwrap();
        let definitions = FilterDefinitions::collect(&document);
        let rect_id = document.node(document.root()).children[1];

        let chain = definitions
            .resolve(document.element(rect_id))
            .unwrap()
            .chain();
        assert_eq!(chain.len(), 1);
        assert_eq!(
            chain[0].kind,
            FilterPrimitiveKind::GaussianBlur(GaussianBlur {
                std_deviation_x: 4.0,
                std_deviation_y: 2.0,
            })
        );
        assert_eq!(chain[0].input, FilterInput::Default);
        assert_eq!(chain[0].input2, FilterInput::Default);
        assert!(chain[0].result.is_none());
    }

    #[test]
    fn filter_reference_accepts_quoted_fragment_urls() {
        assert_eq!(filter_reference_id("url(\"#soft\")"), Some("soft"));
        assert_eq!(filter_reference_id("url('#soft')"), Some("soft"));
        assert_eq!(filter_reference_id("none"), None);
    }

    #[test]
    fn duplicate_filter_ids_keep_the_first_definition() {
        let document = crate::dom::parse(
            r##"<svg><filter id="soft"><feGaussianBlur stdDeviation="2"/></filter><filter id="soft"><feGaussianBlur stdDeviation="8"/></filter><rect filter="url(#soft)"/></svg>"##,
        )
        .unwrap();
        let definitions = FilterDefinitions::collect(&document);
        let rect_id = document.node(document.root()).children[2];

        let chain = definitions
            .resolve(document.element(rect_id))
            .unwrap()
            .chain();
        assert_eq!(chain.len(), 1);
        assert_eq!(
            chain[0].kind,
            FilterPrimitiveKind::GaussianBlur(GaussianBlur {
                std_deviation_x: 2.0,
                std_deviation_y: 2.0,
            })
        );
    }

    #[test]
    fn collect_resolves_url_referenced_fe_image() {
        let document = crate::dom::parse(
            r##"<svg><filter id="tex"><feImage href="data:image/png;base64,abc" x="10%" y="2" width="20" height="50%"/></filter><rect filter="url(#tex)"/></svg>"##,
        )
        .unwrap();
        let definitions = FilterDefinitions::collect(&document);
        let rect_id = document.node(document.root()).children[1];

        let chain = definitions
            .resolve(document.element(rect_id))
            .unwrap()
            .chain();
        assert_eq!(chain.len(), 1);
        assert_eq!(
            chain[0].kind,
            FilterPrimitiveKind::Image(FilterImage {
                href: "data:image/png;base64,abc".to_owned(),
                x: Some(Length::Percent(10.0)),
                y: Some(Length::Px(2.0)),
                width: Some(Length::Px(20.0)),
                height: Some(Length::Percent(50.0)),
                preserve_aspect: PreserveAspect::DEFAULT,
            })
        );
    }

    #[test]
    fn fe_image_accepts_legacy_xlink_href() {
        let document = crate::dom::parse(
            r##"<svg><filter id="tex"><feImage xlink:href="data:image/png;base64,abc"/></filter><rect filter="url(#tex)"/></svg>"##,
        )
        .unwrap();
        let definitions = FilterDefinitions::collect(&document);
        let rect_id = document.node(document.root()).children[1];

        let chain = definitions
            .resolve(document.element(rect_id))
            .unwrap()
            .chain();
        assert!(matches!(
            chain,
            [FilterPrimitive {
                kind: FilterPrimitiveKind::Image(FilterImage { href, .. }),
                ..
            }] if href == "data:image/png;base64,abc"
        ));
    }

    #[test]
    fn gaussian_blur_after_fe_image_blurs_image_source() {
        let document = crate::dom::parse(
            r##"<svg><filter id="tex"><feImage href="data:image/png;base64,abc"/><feGaussianBlur stdDeviation="3"/></filter><rect filter="url(#tex)"/></svg>"##,
        )
        .unwrap();
        let definitions = FilterDefinitions::collect(&document);
        let rect_id = document.node(document.root()).children[1];

        let chain = definitions
            .resolve(document.element(rect_id))
            .unwrap()
            .chain();
        assert_eq!(chain.len(), 2);
        assert!(matches!(
            chain[0].kind,
            FilterPrimitiveKind::Image(FilterImage { .. })
        ));
        assert!(matches!(
            chain[1].kind,
            FilterPrimitiveKind::GaussianBlur(GaussianBlur {
                std_deviation_x: 3.0,
                std_deviation_y: 3.0,
            })
        ));
    }

    #[test]
    fn std_deviation_uses_single_value_for_both_axes() {
        assert_eq!(
            parse_std_deviation(Some("3")),
            Some(GaussianBlur {
                std_deviation_x: 3.0,
                std_deviation_y: 3.0,
            })
        );
    }

    #[test]
    fn std_deviation_rejects_negative_values() {
        assert_eq!(parse_std_deviation(Some("-1")), None);
    }

    #[test]
    fn color_matrix_parses_saturate_shortcut() {
        let document = crate::dom::parse(
            r##"<svg><filter id="cm"><feColorMatrix type="saturate" values="0"/></filter><rect filter="url(#cm)"/></svg>"##,
        )
        .unwrap();
        let definitions = FilterDefinitions::collect(&document);
        let rect_id = document.node(document.root()).children[1];
        let chain = definitions
            .resolve(document.element(rect_id))
            .unwrap()
            .chain();
        // Desaturation maps all RGB channels to the same luma vector.
        let FilterPrimitiveKind::ColorMatrix(cm) = &chain[0].kind else {
            panic!("expected ColorMatrix");
        };
        assert!((cm.matrix[0][0] - 0.213).abs() < 1e-3);
        assert!((cm.matrix[0][1] - 0.715).abs() < 1e-3);
        assert!((cm.matrix[1][0] - 0.213).abs() < 1e-3);
        assert!((cm.matrix[2][0] - 0.213).abs() < 1e-3);
    }

    #[test]
    fn turbulence_parses_default_attributes() {
        let document = crate::dom::parse(
            r##"<svg><filter id="t"><feTurbulence baseFrequency="0.05" numOctaves="3" seed="7" type="fractalNoise"/></filter><rect filter="url(#t)"/></svg>"##,
        )
        .unwrap();
        let definitions = FilterDefinitions::collect(&document);
        let rect_id = document.node(document.root()).children[1];
        let chain = definitions
            .resolve(document.element(rect_id))
            .unwrap()
            .chain();
        let FilterPrimitiveKind::Turbulence(t) = &chain[0].kind else {
            panic!("expected Turbulence");
        };
        assert_eq!(t.base_frequency, [0.05, 0.05]);
        assert_eq!(t.num_octaves, 3);
        assert_eq!(t.seed, 7.0);
        assert!(t.fractal_noise);
    }

    #[test]
    fn morphology_dilate_with_two_radii() {
        let document = crate::dom::parse(
            r##"<svg><filter id="m"><feMorphology operator="dilate" radius="2 3"/></filter><rect filter="url(#m)"/></svg>"##,
        )
        .unwrap();
        let definitions = FilterDefinitions::collect(&document);
        let rect_id = document.node(document.root()).children[1];
        let chain = definitions
            .resolve(document.element(rect_id))
            .unwrap()
            .chain();
        let FilterPrimitiveKind::Morphology(m) = &chain[0].kind else {
            panic!("expected Morphology");
        };
        assert!(m.dilate);
        assert_eq!(m.radius_x, 2.0);
        assert_eq!(m.radius_y, 3.0);
    }

    #[test]
    fn flood_parses_color_and_opacity() {
        let document = crate::dom::parse(
            r##"<svg><filter id="f"><feFlood flood-color="#ff0000" flood-opacity="0.5"/></filter><rect filter="url(#f)"/></svg>"##,
        )
        .unwrap();
        let definitions = FilterDefinitions::collect(&document);
        let rect_id = document.node(document.root()).children[1];
        let chain = definitions
            .resolve(document.element(rect_id))
            .unwrap()
            .chain();
        let FilterPrimitiveKind::Flood(f) = &chain[0].kind else {
            panic!("expected Flood");
        };
        assert!(f.color[0] > 0.9);
        assert!((f.color[3] - 0.5).abs() < 1e-3);
    }

    #[test]
    fn drop_shadow_parses_offset_and_color() {
        let document = crate::dom::parse(
            r##"<svg><filter id="d"><feDropShadow dx="3" dy="4" stdDeviation="2" flood-color="#0000ff"/></filter><rect filter="url(#d)"/></svg>"##,
        )
        .unwrap();
        let definitions = FilterDefinitions::collect(&document);
        let rect_id = document.node(document.root()).children[1];
        let chain = definitions
            .resolve(document.element(rect_id))
            .unwrap()
            .chain();
        let FilterPrimitiveKind::DropShadow(d) = &chain[0].kind else {
            panic!("expected DropShadow");
        };
        assert_eq!(d.offset, [3.0, 4.0]);
        assert_eq!(d.std_deviation_x, 2.0);
        assert!(d.color[2] > 0.9);
    }

    #[test]
    fn displacement_map_defaults_to_alpha_channels() {
        let document = crate::dom::parse(
            r##"<svg><filter id="dm"><feDisplacementMap scale="5"/></filter><rect filter="url(#dm)"/></svg>"##,
        )
        .unwrap();
        let definitions = FilterDefinitions::collect(&document);
        let rect_id = document.node(document.root()).children[1];
        let chain = definitions
            .resolve(document.element(rect_id))
            .unwrap()
            .chain();
        let FilterPrimitiveKind::DisplacementMap(d) = &chain[0].kind else {
            panic!("expected DisplacementMap");
        };
        assert_eq!(d.scale, 5.0);
        assert_eq!(d.x_channel, CH_A);
        assert_eq!(d.y_channel, CH_A);
    }

    #[test]
    fn convolve_matrix_parses_kernel_with_divisor() {
        let document = crate::dom::parse(
            r##"<svg><filter id="c"><feConvolveMatrix kernelMatrix="0 -1 0 -1 5 -1 0 -1 0"/></filter><rect filter="url(#c)"/></svg>"##,
        )
        .unwrap();
        let definitions = FilterDefinitions::collect(&document);
        let rect_id = document.node(document.root()).children[1];
        let chain = definitions
            .resolve(document.element(rect_id))
            .unwrap()
            .chain();
        let FilterPrimitiveKind::ConvolveMatrix(c) = &chain[0].kind else {
            panic!("expected ConvolveMatrix");
        };
        assert_eq!(c.kernel[1][1], 5.0);
        // Default divisor is the kernel sum; here 0+(-1)+0+(-1)+5+(-1)+0+(-1)+0 = 1.
        assert_eq!(c.divisor, 1.0);
    }

    #[test]
    fn component_transfer_picks_per_channel_functions() {
        let document = crate::dom::parse(
            r##"<svg><filter id="ct"><feComponentTransfer><feFuncR type="linear" slope="2" intercept="-0.5"/><feFuncA type="gamma" amplitude="1" exponent="0.5" offset="0"/></feComponentTransfer></filter><rect filter="url(#ct)"/></svg>"##,
        )
        .unwrap();
        let definitions = FilterDefinitions::collect(&document);
        let rect_id = document.node(document.root()).children[1];
        let chain = definitions
            .resolve(document.element(rect_id))
            .unwrap()
            .chain();
        let FilterPrimitiveKind::ComponentTransfer(t) = &chain[0].kind else {
            panic!("expected ComponentTransfer");
        };
        assert_eq!(t.r.kind, FN_LINEAR);
        assert_eq!(t.r.table[..2], [2.0, -0.5]);
        assert_eq!(t.a.kind, FN_GAMMA);
        assert_eq!(t.a.table[..3], [1.0, 0.5, 0.0]);
        assert_eq!(t.g.kind, FN_IDENTITY);
    }

    #[test]
    fn component_transfer_table_preserves_arbitrary_length() {
        // A two-entry `table` is a piecewise-linear interpolation from
        // the first value at C=0 to the second value at C=1 — the
        // identity function when the entries are `0 1`. With the old
        // 4-slot truncation this would have produced nonsense values.
        let document = crate::dom::parse(
            r##"<svg><filter id="t"><feComponentTransfer><feFuncR type="table" tableValues="0 1"/><feFuncG type="discrete" tableValues="0 0.5 1"/></feComponentTransfer></filter><rect filter="url(#t)"/></svg>"##,
        )
        .unwrap();
        let definitions = FilterDefinitions::collect(&document);
        let rect_id = document.node(document.root()).children[1];
        let chain = definitions
            .resolve(document.element(rect_id))
            .unwrap()
            .chain();
        let FilterPrimitiveKind::ComponentTransfer(t) = &chain[0].kind else {
            panic!("expected ComponentTransfer");
        };
        assert_eq!(t.r.kind, FN_TABLE);
        assert_eq!(t.r.count, 2);
        assert_eq!(t.r.table[..2], [0.0, 1.0]);
        assert_eq!(t.g.kind, FN_DISCRETE);
        assert_eq!(t.g.count, 3);
        assert_eq!(t.g.table[..3], [0.0, 0.5, 1.0]);
    }

    #[test]
    fn component_transfer_table_with_one_entry_is_identity() {
        let document = crate::dom::parse(
            r##"<svg><filter id="t"><feComponentTransfer><feFuncR type="table" tableValues="0.5"/></feComponentTransfer></filter><rect filter="url(#t)"/></svg>"##,
        )
        .unwrap();
        let definitions = FilterDefinitions::collect(&document);
        let rect_id = document.node(document.root()).children[1];
        let chain = definitions
            .resolve(document.element(rect_id))
            .unwrap()
            .chain();
        let FilterPrimitiveKind::ComponentTransfer(t) = &chain[0].kind else {
            panic!("expected ComponentTransfer");
        };
        // SVG-1.1 §15.11: a table with fewer than two entries is identity.
        assert_eq!(t.r.kind, FN_IDENTITY);
    }

    #[test]
    fn primitive_chain_preserves_order() {
        let document = crate::dom::parse(
            r##"<svg><filter id="chain"><feGaussianBlur stdDeviation="2"/><feColorMatrix type="saturate" values="0"/></filter><rect filter="url(#chain)"/></svg>"##,
        )
        .unwrap();
        let definitions = FilterDefinitions::collect(&document);
        let rect_id = document.node(document.root()).children[1];
        let chain = definitions
            .resolve(document.element(rect_id))
            .unwrap()
            .chain();
        assert_eq!(chain.len(), 2);
        assert!(matches!(
            chain[0].kind,
            FilterPrimitiveKind::GaussianBlur(_)
        ));
        assert!(matches!(chain[1].kind, FilterPrimitiveKind::ColorMatrix(_)));
    }

    #[test]
    fn primitive_wires_in_in2_and_result_attributes() {
        // The MDN canonical feDisplacementMap fixture: feTurbulence carries
        // `result="turbulence"`; feDisplacementMap sets `in="SourceGraphic"`
        // and `in2="turbulence"`. The parser must preserve all of those
        // wirings so the renderer can build the DAG correctly.
        let document = crate::dom::parse(
            r##"<svg><filter id="warp"><feTurbulence baseFrequency="0.05" numOctaves="2" result="turbulence"/><feDisplacementMap in="SourceGraphic" in2="turbulence" scale="50"/></filter><circle filter="url(#warp)"/></svg>"##,
        )
        .unwrap();
        let definitions = FilterDefinitions::collect(&document);
        let circle_id = document.node(document.root()).children[1];
        let chain = definitions
            .resolve(document.element(circle_id))
            .unwrap()
            .chain();

        assert_eq!(chain.len(), 2);
        assert!(matches!(chain[0].kind, FilterPrimitiveKind::Turbulence(_)));
        assert_eq!(chain[0].result.as_deref(), Some("turbulence"));
        assert!(matches!(
            chain[1].kind,
            FilterPrimitiveKind::DisplacementMap(_)
        ));
        assert_eq!(chain[1].input, FilterInput::SourceGraphic);
        assert_eq!(chain[1].input2, FilterInput::Named("turbulence".to_owned()));
    }

    #[test]
    fn filter_input_parses_known_pseudo_inputs() {
        assert_eq!(FilterInput::parse(None), FilterInput::Default);
        assert_eq!(FilterInput::parse(Some("")), FilterInput::Default);
        assert_eq!(
            FilterInput::parse(Some("SourceGraphic")),
            FilterInput::SourceGraphic
        );
        assert_eq!(
            FilterInput::parse(Some("SourceAlpha")),
            FilterInput::SourceAlpha
        );
        // BackgroundImage / BackgroundAlpha / FillPaint / StrokePaint are
        // recognised pseudo-inputs in their own variants — the renderer
        // currently maps them onto SourceGraphic / SourceAlpha as a
        // documented approximation until filter-local paint and
        // enable-background capture land.
        assert_eq!(
            FilterInput::parse(Some("BackgroundImage")),
            FilterInput::BackgroundImage
        );
        assert_eq!(
            FilterInput::parse(Some("BackgroundAlpha")),
            FilterInput::BackgroundAlpha
        );
        assert_eq!(
            FilterInput::parse(Some("FillPaint")),
            FilterInput::FillPaint
        );
        assert_eq!(
            FilterInput::parse(Some("StrokePaint")),
            FilterInput::StrokePaint
        );
        assert_eq!(
            FilterInput::parse(Some("blurred")),
            FilterInput::Named("blurred".to_owned())
        );
    }

    #[test]
    fn fe_image_rect_defaults_to_intrinsic_size() {
        let image = FilterImage {
            href: "data:image/png;base64,abc".to_owned(),
            x: None,
            y: None,
            width: None,
            height: None,
            preserve_aspect: PreserveAspect::DEFAULT,
        };

        assert_eq!(
            image.resolve_rect(
                Viewport {
                    width: 100.0,
                    height: 80.0,
                },
                12,
                9,
            ),
            Some(ImageRect {
                x: 0.0,
                y: 0.0,
                width: 12.0,
                height: 9.0,
            })
        );
    }
}
