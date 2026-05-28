//! [`ElementKind`] — the tag taxonomy svg3 recognises, with parser and
//! serialiser bridges (`from_tag` / `as_tag`).

/// The kind of an svg3 element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ElementKind {
    /// Document root — the SVG 1.1 `<svg>` element. svg3 documents share
    /// the SVG 1.1 root per [SPEC.md](../../../SPEC.md) §1.1.
    Svg,
    /// Grouping / transform container (SVG 1.1 `<g>`).
    Group,
    /// Definition container (SVG 1.1 `<defs>`).
    Defs,
    /// Axis-aligned rectangle (SVG 1.1 `<rect>` basic shape).
    Rect,
    /// Circle (SVG 1.1 `<circle>` basic shape).
    Circle,
    /// Ellipse (SVG 1.1 `<ellipse>` basic shape).
    Ellipse,
    /// Polygon (SVG 1.1 `<polygon>` basic shape).
    Polygon,
    /// Connected line segments (SVG 1.1 `<polyline>` basic shape).
    Polyline,
    /// Line segment (SVG 1.1 `<line>` basic shape).
    Line,
    /// General path (SVG 1.1 `<path>`).
    Path,
    /// Filter definition container (SVG 1.1 `<filter>`).
    Filter,
    /// Clip-path definition container (SVG 1.1 `<clipPath>`).
    ClipPath,
    /// Mask definition container (SVG 1.1 `<mask>`).
    Mask,
    /// Linear gradient paint server (SVG 1.1 `<linearGradient>`).
    LinearGradient,
    /// Radial gradient paint server (SVG 1.1 `<radialGradient>`).
    RadialGradient,
    /// Gradient stop (SVG 1.1 `<stop>`).
    Stop,
    /// Pattern paint server (SVG 1.1 `<pattern>`).
    Pattern,
    /// Foreign content container (SVG 1.1 `<foreignObject>`). svg3 does not
    /// host HTML, so foreignObject parses but renders as empty.
    ForeignObject,
    /// Gaussian blur filter primitive (SVG 1.1 `<feGaussianBlur>`).
    FeGaussianBlur,
    /// Image filter primitive (SVG 1.1 `<feImage>`).
    FeImage,
    /// Colour matrix filter primitive (SVG 1.1 `<feColorMatrix>`).
    FeColorMatrix,
    /// Turbulence / fractal-noise filter primitive (SVG 1.1 `<feTurbulence>`).
    FeTurbulence,
    /// Specular lighting filter primitive (SVG 1.1 `<feSpecularLighting>`).
    FeSpecularLighting,
    /// Diffuse lighting filter primitive (SVG 1.1 `<feDiffuseLighting>`).
    FeDiffuseLighting,
    /// Morphology (dilate / erode) filter primitive (SVG 1.1 `<feMorphology>`).
    FeMorphology,
    /// Flood fill filter primitive (SVG 1.1 `<feFlood>`).
    FeFlood,
    /// Drop shadow filter primitive (SVG Filter Effects 1 `<feDropShadow>`).
    FeDropShadow,
    /// Displacement map filter primitive (SVG 1.1 `<feDisplacementMap>`).
    FeDisplacementMap,
    /// Convolution matrix filter primitive (SVG 1.1 `<feConvolveMatrix>`).
    FeConvolveMatrix,
    /// Component transfer filter primitive (SVG 1.1 `<feComponentTransfer>`).
    FeComponentTransfer,
    /// Translate-in-pixel-space filter primitive (SVG 1.1 `<feOffset>`).
    FeOffset,
    /// Merge multiple named inputs in painter order (SVG 1.1 `<feMerge>`).
    FeMerge,
    /// One named input in a `<feMerge>` chain (SVG 1.1 `<feMergeNode>`).
    FeMergeNode,
    /// Blend two inputs by mode (SVG 1.1 `<feBlend>`).
    FeBlend,
    /// Composite two inputs by Porter-Duff operator or arithmetic (SVG 1.1
    /// `<feComposite>`).
    FeComposite,
    /// Tile an input across the filter region (SVG 1.1 `<feTile>`).
    FeTile,
    /// Per-channel transfer function for the R channel of `<feComponentTransfer>`.
    FeFuncR,
    /// Per-channel transfer function for the G channel of `<feComponentTransfer>`.
    FeFuncG,
    /// Per-channel transfer function for the B channel of `<feComponentTransfer>`.
    FeFuncB,
    /// Per-channel transfer function for the A channel of `<feComponentTransfer>`.
    FeFuncA,
    /// Directional light source for the lighting filter primitives.
    FeDistantLight,
    /// Point light source for the lighting filter primitives.
    FePointLight,
    /// Spot light source for the lighting filter primitives.
    FeSpotLight,
    /// Marker definition (SVG 1.1 `<marker>`).
    Marker,
    /// Axis-aligned box primitive.
    Cube,
    /// Ellipsoid primitive.
    Ellipsoid,
    /// Right elliptical cylinder primitive.
    Cylinder,
    /// Data-driven 3D surface: a sequence of `<path>` children linked by
    /// Bezier patches described by the surface's own `d` attribute. See
    /// [SPEC.md](../../../SPEC.md) §5.4.
    Surface,
    /// An element not recognised yet (tag name kept verbatim).
    Unknown(String),
}

impl ElementKind {
    /// Map an XML tag name to an [`ElementKind`]. Unrecognised tags are
    /// preserved verbatim so SVG 1.1 markup that svg3 has not implemented
    /// yet (text, masks, …) still round-trips through the DOM.
    pub fn from_tag(tag: &str) -> Self {
        match tag {
            "svg" => Self::Svg,
            "g" => Self::Group,
            "defs" => Self::Defs,
            "rect" => Self::Rect,
            "circle" => Self::Circle,
            "ellipse" => Self::Ellipse,
            "polygon" => Self::Polygon,
            "polyline" => Self::Polyline,
            "line" => Self::Line,
            "path" => Self::Path,
            "filter" => Self::Filter,
            "clipPath" => Self::ClipPath,
            "mask" => Self::Mask,
            "linearGradient" => Self::LinearGradient,
            "radialGradient" => Self::RadialGradient,
            "stop" => Self::Stop,
            "pattern" => Self::Pattern,
            "foreignObject" => Self::ForeignObject,
            "feGaussianBlur" => Self::FeGaussianBlur,
            "feImage" => Self::FeImage,
            "feColorMatrix" => Self::FeColorMatrix,
            "feTurbulence" => Self::FeTurbulence,
            "feSpecularLighting" => Self::FeSpecularLighting,
            "feDiffuseLighting" => Self::FeDiffuseLighting,
            "feMorphology" => Self::FeMorphology,
            "feFlood" => Self::FeFlood,
            "feDropShadow" => Self::FeDropShadow,
            "feDisplacementMap" => Self::FeDisplacementMap,
            "feConvolveMatrix" => Self::FeConvolveMatrix,
            "feComponentTransfer" => Self::FeComponentTransfer,
            "feOffset" => Self::FeOffset,
            "feMerge" => Self::FeMerge,
            "feMergeNode" => Self::FeMergeNode,
            "feBlend" => Self::FeBlend,
            "feComposite" => Self::FeComposite,
            "feTile" => Self::FeTile,
            "feFuncR" => Self::FeFuncR,
            "feFuncG" => Self::FeFuncG,
            "feFuncB" => Self::FeFuncB,
            "feFuncA" => Self::FeFuncA,
            "feDistantLight" => Self::FeDistantLight,
            "fePointLight" => Self::FePointLight,
            "feSpotLight" => Self::FeSpotLight,
            "marker" => Self::Marker,
            "cube" => Self::Cube,
            "ellipsoid" => Self::Ellipsoid,
            "cylinder" => Self::Cylinder,
            "surface" => Self::Surface,
            other => Self::Unknown(other.to_owned()),
        }
    }

    /// The tag name as it appears in source XML.
    pub fn as_tag(&self) -> &str {
        match self {
            Self::Svg => "svg",
            Self::Group => "g",
            Self::Defs => "defs",
            Self::Rect => "rect",
            Self::Circle => "circle",
            Self::Ellipse => "ellipse",
            Self::Polygon => "polygon",
            Self::Polyline => "polyline",
            Self::Line => "line",
            Self::Path => "path",
            Self::Filter => "filter",
            Self::ClipPath => "clipPath",
            Self::Mask => "mask",
            Self::LinearGradient => "linearGradient",
            Self::RadialGradient => "radialGradient",
            Self::Stop => "stop",
            Self::Pattern => "pattern",
            Self::ForeignObject => "foreignObject",
            Self::FeGaussianBlur => "feGaussianBlur",
            Self::FeImage => "feImage",
            Self::FeColorMatrix => "feColorMatrix",
            Self::FeTurbulence => "feTurbulence",
            Self::FeSpecularLighting => "feSpecularLighting",
            Self::FeDiffuseLighting => "feDiffuseLighting",
            Self::FeMorphology => "feMorphology",
            Self::FeFlood => "feFlood",
            Self::FeDropShadow => "feDropShadow",
            Self::FeDisplacementMap => "feDisplacementMap",
            Self::FeConvolveMatrix => "feConvolveMatrix",
            Self::FeComponentTransfer => "feComponentTransfer",
            Self::FeOffset => "feOffset",
            Self::FeMerge => "feMerge",
            Self::FeMergeNode => "feMergeNode",
            Self::FeBlend => "feBlend",
            Self::FeComposite => "feComposite",
            Self::FeTile => "feTile",
            Self::FeFuncR => "feFuncR",
            Self::FeFuncG => "feFuncG",
            Self::FeFuncB => "feFuncB",
            Self::FeFuncA => "feFuncA",
            Self::FeDistantLight => "feDistantLight",
            Self::FePointLight => "fePointLight",
            Self::FeSpotLight => "feSpotLight",
            Self::Marker => "marker",
            Self::Cube => "cube",
            Self::Ellipsoid => "ellipsoid",
            Self::Cylinder => "cylinder",
            Self::Surface => "surface",
            Self::Unknown(t) => t.as_str(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_mapping_recognises_svg3_elements() {
        assert_eq!(ElementKind::from_tag("svg"), ElementKind::Svg);
        assert_eq!(ElementKind::from_tag("g"), ElementKind::Group);
        assert_eq!(ElementKind::from_tag("rect"), ElementKind::Rect);
        assert_eq!(ElementKind::from_tag("circle"), ElementKind::Circle);
        assert_eq!(ElementKind::from_tag("ellipse"), ElementKind::Ellipse);
        assert_eq!(ElementKind::from_tag("polygon"), ElementKind::Polygon);
        assert_eq!(ElementKind::from_tag("polyline"), ElementKind::Polyline);
        assert_eq!(ElementKind::from_tag("line"), ElementKind::Line);
        assert_eq!(ElementKind::from_tag("path"), ElementKind::Path);
        assert_eq!(ElementKind::from_tag("filter"), ElementKind::Filter);
        assert_eq!(
            ElementKind::from_tag("feGaussianBlur"),
            ElementKind::FeGaussianBlur
        );
        assert_eq!(ElementKind::from_tag("feImage"), ElementKind::FeImage);
        assert_eq!(
            ElementKind::from_tag("feColorMatrix"),
            ElementKind::FeColorMatrix
        );
        assert_eq!(
            ElementKind::from_tag("feTurbulence"),
            ElementKind::FeTurbulence
        );
        assert_eq!(
            ElementKind::from_tag("feSpecularLighting"),
            ElementKind::FeSpecularLighting
        );
        assert_eq!(
            ElementKind::from_tag("feDiffuseLighting"),
            ElementKind::FeDiffuseLighting
        );
        assert_eq!(
            ElementKind::from_tag("feMorphology"),
            ElementKind::FeMorphology
        );
        assert_eq!(ElementKind::from_tag("feFlood"), ElementKind::FeFlood);
        assert_eq!(
            ElementKind::from_tag("feDropShadow"),
            ElementKind::FeDropShadow
        );
        assert_eq!(
            ElementKind::from_tag("feDisplacementMap"),
            ElementKind::FeDisplacementMap
        );
        assert_eq!(
            ElementKind::from_tag("feConvolveMatrix"),
            ElementKind::FeConvolveMatrix
        );
        assert_eq!(
            ElementKind::from_tag("feComponentTransfer"),
            ElementKind::FeComponentTransfer
        );
        assert_eq!(ElementKind::from_tag("feOffset"), ElementKind::FeOffset);
        assert_eq!(ElementKind::from_tag("feMerge"), ElementKind::FeMerge);
        assert_eq!(
            ElementKind::from_tag("feMergeNode"),
            ElementKind::FeMergeNode
        );
        assert_eq!(ElementKind::from_tag("feBlend"), ElementKind::FeBlend);
        assert_eq!(
            ElementKind::from_tag("feComposite"),
            ElementKind::FeComposite
        );
        assert_eq!(ElementKind::from_tag("feTile"), ElementKind::FeTile);
        assert_eq!(ElementKind::from_tag("feFuncR"), ElementKind::FeFuncR);
        assert_eq!(ElementKind::from_tag("feFuncA"), ElementKind::FeFuncA);
        assert_eq!(
            ElementKind::from_tag("feDistantLight"),
            ElementKind::FeDistantLight
        );
        assert_eq!(
            ElementKind::from_tag("fePointLight"),
            ElementKind::FePointLight
        );
        assert_eq!(
            ElementKind::from_tag("feSpotLight"),
            ElementKind::FeSpotLight
        );
        assert_eq!(ElementKind::from_tag("cube"), ElementKind::Cube);
        assert_eq!(ElementKind::from_tag("ellipsoid"), ElementKind::Ellipsoid);
        assert_eq!(ElementKind::from_tag("cylinder"), ElementKind::Cylinder);
        assert_eq!(ElementKind::from_tag("surface"), ElementKind::Surface);
        assert_eq!(ElementKind::Cylinder.as_tag(), "cylinder");
        assert_eq!(ElementKind::Surface.as_tag(), "surface");
        assert_eq!(ElementKind::Rect.as_tag(), "rect");
        assert_eq!(ElementKind::Ellipse.as_tag(), "ellipse");
        assert_eq!(ElementKind::Polygon.as_tag(), "polygon");
        assert_eq!(ElementKind::Polyline.as_tag(), "polyline");
        assert_eq!(ElementKind::Line.as_tag(), "line");
        assert_eq!(ElementKind::Path.as_tag(), "path");
        assert_eq!(ElementKind::Filter.as_tag(), "filter");
        assert_eq!(ElementKind::FeGaussianBlur.as_tag(), "feGaussianBlur");
        assert_eq!(ElementKind::FeImage.as_tag(), "feImage");
        assert_eq!(ElementKind::FeColorMatrix.as_tag(), "feColorMatrix");
        assert_eq!(ElementKind::FeFlood.as_tag(), "feFlood");
        assert_eq!(ElementKind::FeDropShadow.as_tag(), "feDropShadow");
        assert_eq!(
            ElementKind::FeComponentTransfer.as_tag(),
            "feComponentTransfer"
        );
        assert_eq!(ElementKind::FeFuncR.as_tag(), "feFuncR");
        assert_eq!(ElementKind::FeDistantLight.as_tag(), "feDistantLight");
        // `<group>` is not in SPEC.md; only `<g>` from SVG 1.1 is the
        // canonical grouping element.
        assert_eq!(
            ElementKind::from_tag("group"),
            ElementKind::Unknown("group".to_owned())
        );
        // Similarly, `<scene>` (an earlier draft name) is no longer
        // recognised — it round-trips as Unknown.
        assert_eq!(
            ElementKind::from_tag("scene"),
            ElementKind::Unknown("scene".to_owned())
        );
        assert_eq!(
            ElementKind::from_tag("widget"),
            ElementKind::Unknown("widget".to_owned())
        );
    }
}
