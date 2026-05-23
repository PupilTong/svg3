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
//! in [`crate::renderer`] and `filter.wgsl`.

use std::collections::BTreeMap;

use svg3_dom::{Document, Element, ElementKind, NodeId};

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
    /// Kind-specific parameters.
    pub(crate) kind: FilterPrimitiveKind,
}

/// The kind-specific data carried by a [`FilterPrimitive`].
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum FilterPrimitiveKind {
    /// Gaussian blur — two separable passes.
    GaussianBlur(GaussianBlur),
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
}

impl FilterPrimitive {
    /// Whether this primitive changes its input. Used to skip no-op chains.
    pub(crate) fn is_visible(&self) -> bool {
        match &self.kind {
            FilterPrimitiveKind::GaussianBlur(blur) => blur.is_visible(),
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
    /// A named earlier primitive's `result`.
    Named(String),
}

impl FilterInput {
    /// Parse an `in` / `in2` attribute value. Unsupported pseudo-inputs
    /// (`BackgroundImage`, `BackgroundAlpha`, `FillPaint`, `StrokePaint`)
    /// fall back to the SVG default per spec, so a document still renders
    /// rather than panicking.
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
            // Unsupported pseudo-inputs fall back to "Default" (SVG-spec
            // behaviour for unresolvable references).
            "BackgroundImage" | "BackgroundAlpha" | "FillPaint" | "StrokePaint" => Self::Default,
            other => Self::Named(other.to_owned()),
        }
    }
}

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
}

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

/// Filter definitions keyed by their XML `id` attribute.
#[derive(Debug, Default)]
pub(crate) struct FilterDefinitions {
    filters: BTreeMap<String, Vec<FilterPrimitive>>,
}

impl FilterDefinitions {
    /// Collect supported filter definitions from the whole document.
    ///
    /// If duplicate `id` values appear, the first supported definition wins,
    /// matching SVG's first-element lookup behavior for fragment references.
    pub(crate) fn collect(document: &Document) -> Self {
        let mut definitions = Self::default();
        let mut stack = vec![document.root()];
        while let Some(id) = stack.pop() {
            let node = document.node(id);
            if node.element.kind == ElementKind::Filter {
                if let Some(filter_id) = node.element.attributes.get("id") {
                    let chain = collect_primitives(document, node.children.iter().copied());
                    if !chain.is_empty() {
                        definitions
                            .filters
                            .entry(filter_id.to_owned())
                            .or_insert(chain);
                    }
                }
                continue;
            }
            stack.extend(node.children.iter().rev().copied());
        }
        definitions
    }

    /// Resolve an element's `filter="url(#id)"` presentation attribute.
    ///
    /// Returns the parsed primitive chain, or `None` when the reference is
    /// absent, unknown, or expanded to an empty chain.
    pub(crate) fn resolve(&self, element: &Element) -> Option<&[FilterPrimitive]> {
        let id = element
            .attributes
            .get("filter")
            .and_then(|value| filter_reference_id(value))?;
        self.filters.get(id).map(Vec::as_slice)
    }
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

fn resolve_primitive(document: &Document, node: &svg3_dom::Node) -> Option<FilterPrimitive> {
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
        kind,
    })
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

fn parse_lighting(document: &Document, node: &svg3_dom::Node, specular: bool) -> Lighting {
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
    let lighting_color = node
        .element
        .attributes
        .get("lighting-color")
        .and_then(|s| crate::shapes::parse_color_value(s))
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
    let mut color = element
        .attributes
        .get("flood-color")
        .and_then(|value| crate::shapes::parse_color_value(value))
        .unwrap_or([0.0, 0.0, 0.0, 1.0]);
    if let Some(opacity) = element
        .attributes
        .get("flood-opacity")
        .and_then(|value| value.trim().parse::<f32>().ok())
    {
        color[3] *= opacity.clamp(0.0, 1.0);
    }
    Flood { color }
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
    let mut color = element
        .attributes
        .get("flood-color")
        .and_then(|value| crate::shapes::parse_color_value(value))
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
    ConvolveMatrix {
        kernel,
        divisor,
        bias,
        preserve_alpha,
    }
}

fn parse_component_transfer(document: &Document, node: &svg3_dom::Node) -> ComponentTransfer {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_resolves_url_referenced_gaussian_blur() {
        let document = svg3_dom::parse(
            r##"<svg><filter id="soft"><feGaussianBlur stdDeviation="4 2"/></filter><rect filter="url(#soft)"/></svg>"##,
        )
        .unwrap();
        let definitions = FilterDefinitions::collect(&document);
        let rect_id = document.node(document.root()).children[1];

        let chain = definitions.resolve(document.element(rect_id)).unwrap();
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
        let document = svg3_dom::parse(
            r##"<svg><filter id="soft"><feGaussianBlur stdDeviation="2"/></filter><filter id="soft"><feGaussianBlur stdDeviation="8"/></filter><rect filter="url(#soft)"/></svg>"##,
        )
        .unwrap();
        let definitions = FilterDefinitions::collect(&document);
        let rect_id = document.node(document.root()).children[2];

        let chain = definitions.resolve(document.element(rect_id)).unwrap();
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
        let document = svg3_dom::parse(
            r##"<svg><filter id="cm"><feColorMatrix type="saturate" values="0"/></filter><rect filter="url(#cm)"/></svg>"##,
        )
        .unwrap();
        let definitions = FilterDefinitions::collect(&document);
        let rect_id = document.node(document.root()).children[1];
        let chain = definitions.resolve(document.element(rect_id)).unwrap();
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
        let document = svg3_dom::parse(
            r##"<svg><filter id="t"><feTurbulence baseFrequency="0.05" numOctaves="3" seed="7" type="fractalNoise"/></filter><rect filter="url(#t)"/></svg>"##,
        )
        .unwrap();
        let definitions = FilterDefinitions::collect(&document);
        let rect_id = document.node(document.root()).children[1];
        let chain = definitions.resolve(document.element(rect_id)).unwrap();
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
        let document = svg3_dom::parse(
            r##"<svg><filter id="m"><feMorphology operator="dilate" radius="2 3"/></filter><rect filter="url(#m)"/></svg>"##,
        )
        .unwrap();
        let definitions = FilterDefinitions::collect(&document);
        let rect_id = document.node(document.root()).children[1];
        let chain = definitions.resolve(document.element(rect_id)).unwrap();
        let FilterPrimitiveKind::Morphology(m) = &chain[0].kind else {
            panic!("expected Morphology");
        };
        assert!(m.dilate);
        assert_eq!(m.radius_x, 2.0);
        assert_eq!(m.radius_y, 3.0);
    }

    #[test]
    fn flood_parses_color_and_opacity() {
        let document = svg3_dom::parse(
            r##"<svg><filter id="f"><feFlood flood-color="#ff0000" flood-opacity="0.5"/></filter><rect filter="url(#f)"/></svg>"##,
        )
        .unwrap();
        let definitions = FilterDefinitions::collect(&document);
        let rect_id = document.node(document.root()).children[1];
        let chain = definitions.resolve(document.element(rect_id)).unwrap();
        let FilterPrimitiveKind::Flood(f) = &chain[0].kind else {
            panic!("expected Flood");
        };
        assert!(f.color[0] > 0.9);
        assert!((f.color[3] - 0.5).abs() < 1e-3);
    }

    #[test]
    fn drop_shadow_parses_offset_and_color() {
        let document = svg3_dom::parse(
            r##"<svg><filter id="d"><feDropShadow dx="3" dy="4" stdDeviation="2" flood-color="#0000ff"/></filter><rect filter="url(#d)"/></svg>"##,
        )
        .unwrap();
        let definitions = FilterDefinitions::collect(&document);
        let rect_id = document.node(document.root()).children[1];
        let chain = definitions.resolve(document.element(rect_id)).unwrap();
        let FilterPrimitiveKind::DropShadow(d) = &chain[0].kind else {
            panic!("expected DropShadow");
        };
        assert_eq!(d.offset, [3.0, 4.0]);
        assert_eq!(d.std_deviation_x, 2.0);
        assert!(d.color[2] > 0.9);
    }

    #[test]
    fn displacement_map_defaults_to_alpha_channels() {
        let document = svg3_dom::parse(
            r##"<svg><filter id="dm"><feDisplacementMap scale="5"/></filter><rect filter="url(#dm)"/></svg>"##,
        )
        .unwrap();
        let definitions = FilterDefinitions::collect(&document);
        let rect_id = document.node(document.root()).children[1];
        let chain = definitions.resolve(document.element(rect_id)).unwrap();
        let FilterPrimitiveKind::DisplacementMap(d) = &chain[0].kind else {
            panic!("expected DisplacementMap");
        };
        assert_eq!(d.scale, 5.0);
        assert_eq!(d.x_channel, CH_A);
        assert_eq!(d.y_channel, CH_A);
    }

    #[test]
    fn convolve_matrix_parses_kernel_with_divisor() {
        let document = svg3_dom::parse(
            r##"<svg><filter id="c"><feConvolveMatrix kernelMatrix="0 -1 0 -1 5 -1 0 -1 0"/></filter><rect filter="url(#c)"/></svg>"##,
        )
        .unwrap();
        let definitions = FilterDefinitions::collect(&document);
        let rect_id = document.node(document.root()).children[1];
        let chain = definitions.resolve(document.element(rect_id)).unwrap();
        let FilterPrimitiveKind::ConvolveMatrix(c) = &chain[0].kind else {
            panic!("expected ConvolveMatrix");
        };
        assert_eq!(c.kernel[1][1], 5.0);
        // Default divisor is the kernel sum; here 0+(-1)+0+(-1)+5+(-1)+0+(-1)+0 = 1.
        assert_eq!(c.divisor, 1.0);
    }

    #[test]
    fn component_transfer_picks_per_channel_functions() {
        let document = svg3_dom::parse(
            r##"<svg><filter id="ct"><feComponentTransfer><feFuncR type="linear" slope="2" intercept="-0.5"/><feFuncA type="gamma" amplitude="1" exponent="0.5" offset="0"/></feComponentTransfer></filter><rect filter="url(#ct)"/></svg>"##,
        )
        .unwrap();
        let definitions = FilterDefinitions::collect(&document);
        let rect_id = document.node(document.root()).children[1];
        let chain = definitions.resolve(document.element(rect_id)).unwrap();
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
        let document = svg3_dom::parse(
            r##"<svg><filter id="t"><feComponentTransfer><feFuncR type="table" tableValues="0 1"/><feFuncG type="discrete" tableValues="0 0.5 1"/></feComponentTransfer></filter><rect filter="url(#t)"/></svg>"##,
        )
        .unwrap();
        let definitions = FilterDefinitions::collect(&document);
        let rect_id = document.node(document.root()).children[1];
        let chain = definitions.resolve(document.element(rect_id)).unwrap();
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
        let document = svg3_dom::parse(
            r##"<svg><filter id="t"><feComponentTransfer><feFuncR type="table" tableValues="0.5"/></feComponentTransfer></filter><rect filter="url(#t)"/></svg>"##,
        )
        .unwrap();
        let definitions = FilterDefinitions::collect(&document);
        let rect_id = document.node(document.root()).children[1];
        let chain = definitions.resolve(document.element(rect_id)).unwrap();
        let FilterPrimitiveKind::ComponentTransfer(t) = &chain[0].kind else {
            panic!("expected ComponentTransfer");
        };
        // SVG-1.1 §15.11: a table with fewer than two entries is identity.
        assert_eq!(t.r.kind, FN_IDENTITY);
    }

    #[test]
    fn primitive_chain_preserves_order() {
        let document = svg3_dom::parse(
            r##"<svg><filter id="chain"><feGaussianBlur stdDeviation="2"/><feColorMatrix type="saturate" values="0"/></filter><rect filter="url(#chain)"/></svg>"##,
        )
        .unwrap();
        let definitions = FilterDefinitions::collect(&document);
        let rect_id = document.node(document.root()).children[1];
        let chain = definitions.resolve(document.element(rect_id)).unwrap();
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
        let document = svg3_dom::parse(
            r##"<svg><filter id="warp"><feTurbulence baseFrequency="0.05" numOctaves="2" result="turbulence"/><feDisplacementMap in="SourceGraphic" in2="turbulence" scale="50"/></filter><circle filter="url(#warp)"/></svg>"##,
        )
        .unwrap();
        let definitions = FilterDefinitions::collect(&document);
        let circle_id = document.node(document.root()).children[1];
        let chain = definitions.resolve(document.element(circle_id)).unwrap();

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
        // Unsupported pseudo-inputs fall back to Default per SVG semantics.
        assert_eq!(
            FilterInput::parse(Some("BackgroundImage")),
            FilterInput::Default
        );
        assert_eq!(
            FilterInput::parse(Some("blurred")),
            FilterInput::Named("blurred".to_owned())
        );
    }
}
