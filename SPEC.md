# svg3 Specification — Draft 0

**Status:** working draft. The svg3 implementation is at an early scaffold (see
[README.md](README.md) roadmap). This document is **descriptive of intent and
normative for what already ships**, marked per section. It will firm up as the
pipeline (parse → style → render) reaches feature parity with the spec.

**Scope:** the on-the-wire document language and the rendering model. Public
Rust API surface, build/run instructions, and contributor conventions live in
[README.md](README.md) and [AGENTS.md](AGENTS.md).

svg3 is a **superset of SVG 1.1**: the root is the SVG 1.1 `<svg>` element,
SVG 1.1's 2D drawing model is inherited wholesale, and 3D content is added
inside a new `<scene>` child element. A 2D-only SVG 1.1 renderer encountering
an svg3 document renders the SVG 1.1 parts and ignores `<scene>` as an unknown
element — useful graceful degradation. See
[§10. Relationship to SVG 1.1](#10-relationship-to-svg-11) for the full
mapping. Canonical references:

- [SVG 1.1 (W3C)](https://www.w3.org/TR/SVG11/) — inherited wholesale; this
  spec does not re-specify SVG 1.1 features, it points at them.
- [CSS Transforms Module Level 2](https://www.w3.org/TR/css-transforms-2/) —
  3D transform function syntax and composition rules (for content inside
  `<scene>`).
- [CSS Color Module Level 4](https://www.w3.org/TR/css-color-4/) — paint
  values.
- The host CSS engine for everything else: svg3 resolves styles through
  [Stylo](https://crates.io/crates/stylo) and inherits its CSS semantics
  directly (see [§8. Styling](#8-styling)).

---

## 1. Introduction

svg3 is an **XML-based document language that extends SVG 1.1 with
three-dimensional content**. The root is the SVG 1.1 `<svg>` element.
Inside that root, a `<scene>` child introduces a 3D coordinate system
on a region of the 2D canvas; 3D primitives (`<cube>`, `<ellipsoid>`,
…) live inside `<scene>`. SVG 1.1's 2D model — paths, basic shapes,
text, the full SVG 1.1 chapter list — applies normally outside
`<scene>`, and an svg3 implementation will eventually be a conforming
SVG 1.1 renderer too.

A document describes geometry only. The host application supplies the
rendering surface and, for 3D content, the camera (see
[§7. Camera and viewport](#7-camera-and-viewport)); the document does
not schedule animation in v0.

### 1.1 Why extend SVG instead of forking

SVG 1.1 already provides what a 3D scene description needs as
infrastructure: an XML tree, a mature attribute / styling vocabulary,
CSS cascade semantics, and authoring ergonomics designers know.
Rather than fork a new format, svg3 keeps `<svg>` as the root and adds
3D inside a dedicated `<scene>` element so a document can mix 2D and
3D content cleanly.

The `<scene>` boundary gives a clean graceful-degradation story:
a 2D-only SVG 1.1 renderer treats `<scene>` as an unknown element and
skips it, while still drawing whatever 2D content the document
carries alongside.

svg3 is **not** a submission to the SVG WG and is not intended to
render inside a browser's native SVG engine; the native svg3 renderer
is the conformance target. Compatibility with SVG 1.1 is about
authoring intuition and ecosystem tooling, not browser support.

### 1.2 Reading this document

Each non-trivial section opens with a status line of the form
`**Status:** [Foo]` directly under the heading:

- **\[Implemented\]** — the current scaffold meets this section.
- **\[Partial\]** — parts shipped, parts pending; the section notes which.
- **\[Roadmap\]** — design recorded here, no code yet.
- **\[Out of scope (v0)\]** — explicitly not in v0; may return in a future
  draft.

The tag sits outside the heading so anchor slugs (`#8-styling` etc.)
stay stable as status moves from Roadmap → Partial → Implemented.

---

## 2. Status and conformance

**Status:** \[Roadmap\]

svg3 conformance has two layered targets:

- **SVG 1.1 conformance.** A conforming svg3 implementation also
  conforms to SVG 1.1 for the SVG 1.1 surface. svg3 inherits SVG 1.1's
  semantics by reference and does not re-specify them. The v0
  implementation does **not** meet this yet; the v0 parser only knows
  the 3D-specific elements and `<g>`. Coverage of the rest of SVG 1.1
  is on the roadmap ([§10.3](#103-svg-11-implementation-status)).
- **3D-extension conformance** — defined below for the svg3-only surface.

There is no conformance suite yet. When v0.1 of the implementation
ships (roadmap items 3–4 in [README.md](README.md): mesh generation,
render a single cube, real Stylo cascade), 3D-extension conformance
will be defined as:

1. **Parser conformance** — accept every well-formed document this spec
   describes, reject documents this spec disallows, surface typed errors
   as listed in [`svg3_dom::ParseError`](svg3-dom/src/lib.rs).
2. **Style conformance** — produce the same computed-style values for a
   document as Stylo produces for the equivalent CSS rules against the
   same element tree.
3. **Render conformance** — produce a frame whose visible content matches
   a reference image within a perceptual tolerance, for a fixed corpus of
   test documents. The corpus does not exist yet.

This document does not yet define error-recovery behavior beyond the
typed errors in `svg3-dom`; the reference parser is fail-fast. SVG 1.1
is **not** fail-fast, so error-recovery alignment is a known open
question — see [§11](#11-open-questions-and-future-drafts).

> **Implementation note (Draft 0).** [`svg3_dom::parse`](svg3-dom/src/lib.rs)
> currently hard-codes `<scene>` as the required root and rejects
> `<svg>`-rooted documents with `ParseError::UnexpectedRoot`. This is
> out of step with this draft and is tracked as item 1 in
> [§11](#11-open-questions-and-future-drafts).

---

## 3. Document syntax

**Status:** \[Implemented (with the root mismatch noted in §2)\]

### 3.1 Wire format

A svg3 document is an XML 1.0 text document. The reference parser is
[`svg3_dom::parse`](svg3-dom/src/lib.rs) (built on
[`quick-xml`](https://crates.io/crates/quick-xml)). The following XML
constructs are recognised:

| Construct | Handling |
|---|---|
| Start tag (`<svg>`) | begins a new element |
| Empty-element tag (`<cube/>`) | begins and ends an element |
| End tag (`</svg>`) | closes the current element |
| Attributes (`name="value"`) | stored verbatim after XML-unescape |
| Text, CDATA, comments, PIs, DOCTYPE, XML decl | **ignored** in v0 (text is on the roadmap once SVG 1.1 `<text>` lands) |
| Namespaces | **not handled** in v0 (prefixes are part of the tag name) |

The mandatory root element is `<svg>` — same as SVG 1.1 ch. 5.

### 3.2 Attribute values

Attribute values are stored as raw `String`s on the owning element. The
parser does **not** interpret them into typed forms; that responsibility
sits with the layer that consumes the value (the style cascade for
`class`/`id`/`style`, the renderer for geometry attributes like `size`).

Rationale: the same string needs to flow into Stylo (which expects raw
attribute text for selector matching) and into the renderer (which parses
its own typed form). One canonical representation, parsed at the point of
use, avoids double-conversion. The reference DOM has shipped exactly this
shape since [`svg3-dom#4`](https://github.com/PupilTong/svg3/pull/4).

### 3.3 Document model

`svg3-dom` exposes a flat arena: nodes live in a `Vec<Node>` keyed by
`NodeId`, each node owning a `Vec<NodeId>` of children. The root is
always `NodeId(0)`. This shape (rather than `Rc<RefCell<…>>` or `Box`-ed
trees) is chosen so the Stylo cascade — modelled on
[Blitz's `blitz-dom`](https://github.com/DioxusLabs/blitz) — can walk the
tree with cheap, stable identifiers and without fighting the borrow
checker. See [`svg3-dom/src/lib.rs`](svg3-dom/src/lib.rs) for the public
API.

### 3.4 MIME type and file extension

The conventional file extension is `.svg`. svg3 is a strict superset of
SVG 1.1, so an svg3 document with no `<scene>` content is also a valid
SVG 1.1 document and round-trips through SVG-aware tools. MIME-type
registration is not in scope for this draft.

---

## 4. Elements

### 4.1 Element list (v0)

| Tag | Kind | Status | Section |
|---|---|---|---|
| `<svg>` | root container (SVG 1.1) | implemented (parse only; root not yet enforced — see §2) | [§4.2](#42-svg) |
| `<scene>` | 3D-context container | implemented (parse only) | [§4.3](#43-scene) |
| `<g>`, `<group>` | grouping / transform | implemented (parse only) | [§4.4](#44-g--group) |
| `<cube>` | axis-aligned box primitive | implemented (parse only) | [§4.5](#45-cube) |
| `<ellipsoid>` | ellipsoid primitive | implemented (parse only) | [§4.6](#46-ellipsoid) |

The remaining SVG 1.1 elements (`<path>`, `<rect>`, `<circle>`,
`<ellipse>`, `<line>`, `<polyline>`, `<polygon>`, `<text>`, `<defs>`,
`<use>`, `<symbol>`, …) are part of svg3 by inheritance from SVG 1.1
(see [§10](#10-relationship-to-svg-11)) but **not implemented in v0**.
The parser preserves any tag it does not recognise as
`ElementKind::Unknown` so SVG 1.1 markup round-trips through the DOM
without losing information.

Attributes named in this section are the v0 set. Any other attribute is
preserved on the element (for the cascade's benefit — `id`, `class`,
`style`, and any author-defined attribute can flow through), but only
the listed attributes affect rendering.

### 4.2 `<svg>`

**Status:** \[Roadmap (full SVG 1.1 semantics); root parsing tracked in §2\]

The document root, matching SVG 1.1 ch. 5. The full set of SVG 1.1
attributes (`width`, `height`, `viewBox`, `preserveAspectRatio`, `x`,
`y`, `version`, `baseProfile`, …) applies by inheritance from SVG 1.1.

`<svg>` establishes the document's outer 2D coordinate system (SVG 1.1's
+Y-down user space). 3D content goes inside one or more `<scene>`
children ([§4.3](#43-scene)); 2D SVG 1.1 content lives directly under
`<svg>` or inside `<g>`.

### 4.3 `<scene>`

**Status:** \[Partial — parse implemented, 3D semantics on roadmap\]

`<scene>` is a child of `<svg>` (or of an SVG 1.1 grouping element)
that introduces a **3D rendering context**: coordinates inside
`<scene>` are interpreted by [§5. Coordinate system](#5-coordinate-system-and-units),
and transforms use the 3D function set ([§6.2](#62-3d-transform-functions)).
The `<scene>` element is the boundary between SVG 1.1's 2D world and
svg3's 3D world.

Attributes affecting rendering in v0: none. The renderer treats the
`<svg>` viewport as the 3D viewport when a single `<scene>` is the sole
rendered child. Per-`<scene>` placement (rendering a `<scene>` into a
sub-rectangle of the canvas) is on the roadmap; the natural hook is SVG
1.1's `x` / `y` / `width` / `height` attribute set — the same attributes
nested `<svg>` and `<foreignObject>` use.

Nested `<scene>` inside `<scene>` is undefined in v0 — see
[§11](#11-open-questions-and-future-drafts).

### 4.4 `<g>` / `<group>`

**Status:** \[Implemented (parse only)\]

Grouping element. Equivalent to SVG 1.1's `<g>`. Both spellings are
accepted; the canonical tag is `<g>`. Permitted attributes:

- `transform` — see [§6. Transforms](#6-transforms). The function set
  available depends on whether the `<g>` is inside `<scene>` (3D
  function set) or outside (SVG 1.1 2D function set).
- `id`, `class`, `style` — see [§8. Styling](#8-styling).

A `<g>` is a transform/style boundary; it has no intrinsic geometry.

### 4.5 `<cube>`

**Status:** \[Roadmap (geometry)\]

An axis-aligned box centered at the local origin. Valid only inside a
`<scene>` element; outside `<scene>` it parses successfully but does
not render.

| Attribute | Type | Default | Meaning |
|---|---|---|---|
| `size` | length | `1` | uniform edge length (sets `width`/`height`/`depth`) |
| `width` | length | from `size` | extent along local +X |
| `height` | length | from `size` | extent along local +Y |
| `depth` | length | from `size` | extent along local +Z |
| `transform` | transform-list | identity | see [§6](#6-transforms) |
| `id`, `class`, `style` | — | — | see [§8](#8-styling) |

If `size` and any of `width`/`height`/`depth` are both present, the
per-axis attribute wins.

### 4.6 `<ellipsoid>`

**Status:** \[Roadmap (geometry)\]

An ellipsoid centered at the local origin. Valid only inside a
`<scene>` element; outside `<scene>` it parses successfully but does
not render.

| Attribute | Type | Default | Meaning |
|---|---|---|---|
| `r` | length | `1` | uniform radius (sets `rx`/`ry`/`rz`) |
| `rx` | length | from `r` | radius along local X |
| `ry` | length | from `r` | radius along local Y |
| `rz` | length | from `r` | radius along local Z |
| `transform` | transform-list | identity | see [§6](#6-transforms) |
| `id`, `class`, `style` | — | — | see [§8](#8-styling) |

If `r` and any of `rx`/`ry`/`rz` are both present, the per-axis attribute
wins. An `<ellipsoid>` with `rx=ry=rz` is a sphere.

---

## 5. Coordinate system and units

**Status:** \[Roadmap\]

### 5.1 Two coordinate systems

svg3 documents have two coordinate systems that share the same XML
tree:

- **2D space — outside `<scene>`.** SVG 1.1's coordinate system:
  +X right, +Y down, user units. Governs all SVG 1.1 elements (paths,
  basic shapes, text, …). Inherited unchanged from SVG 1.1 ch. 7.
- **3D space — inside `<scene>`.** Right-handed, +X right, **+Y up**,
  +Z toward the viewer; user units; host-supplied camera. Governs
  svg3's 3D primitives.

The `<scene>` element marks the transition: the 3D camera projects the
3D scene into the `<scene>`'s 2D rectangle, and that rectangle then
composites with surrounding 2D content per SVG 1.1's painter's
algorithm.

The rest of this section describes the 3D space; the 2D space is
specified by SVG 1.1 and inherited unchanged.

### 5.2 Spaces

Inside `<scene>`:

- **Object space.** Each primitive is defined relative to a local origin
  (the centroid for `<cube>` and `<ellipsoid>`).
- **Local space.** Object space with the element's `transform` applied.
- **World space.** Local space with all ancestor `transform`s applied,
  composing from `<scene>` down to leaf.
- **View space, clip space.** Owned by the camera (see
  [§7.2](#72-the-3d-camera)); not addressable from the document.

### 5.3 Handedness and axes

Inside `<scene>`, svg3 is **right-handed** with:

- **+X** to the right,
- **+Y** up,
- **+Z** toward the viewer (out of the screen).

This matches the convention in [`svg3_render::Camera`](svg3-render/src/lib.rs)
(`glam::Mat4::look_at_rh` with `Vec3::Y` as up) and the prevailing
convention in 3D graphics tooling.

This is a **deliberate divergence from SVG 1.1's +Y-down 2D system**.
The visual-debugging cost of carrying +Y-down into a 3D scene (every
rotation reading mirrored) outweighs the consistency benefit of
matching the 2D space; the `<scene>` boundary is the natural place to
flip Y.

### 5.4 Units

Inside `<scene>`, a "user unit" is the base length. There are no
`px`/`em`/`pt` qualifiers in v0 — `size="2"` is interpreted as `2` user
units, full stop. The camera projection and the framebuffer resolution
determine how many pixels a user unit occupies on screen.

Lengths are parsed as IEEE-754 `f32`. Negative lengths are not allowed
for radii or extents (`size`, `r`, `width`, `height`, `depth`, `rx`,
`ry`, `rz`).

CSS-style length units (`%`, `em`, `vh`, …) are **out of scope for v0**
on 3D content. Units for 2D content outside `<scene>` follow SVG 1.1.

### 5.5 Angles

Angles in `transform` functions accept the following suffixes (matching
[CSS Values 4](https://www.w3.org/TR/css-values-4/#angles)):

- `deg` — degrees (default if the suffix is omitted)
- `rad` — radians
- `turn` — full turns (`1turn` = `360deg`)
- `grad` — gradians

---

## 6. Transforms

**Status:** \[Roadmap\]

### 6.1 The `transform` attribute

The `transform` attribute is permitted on elements that carry a
coordinate space (`<g>`, `<cube>`, `<ellipsoid>`, `<scene>`, and the
SVG 1.1 elements that already accept it). The **function set permitted
depends on context**:

- **Outside `<scene>` (2D, SVG 1.1):** the SVG 1.1 / CSS Transforms 1
  function set — `matrix`, `translate`, `scale`, `rotate`, `skewX`,
  `skewY`. See [SVG 1.1 ch. 7](https://www.w3.org/TR/SVG11/coords.html).
- **Inside `<scene>` (3D, svg3):** the 3D function set listed in
  [§6.2](#62-3d-transform-functions), matching CSS Transforms 2.

The `transform` on `<scene>` itself is interpreted in **2D**: it
positions the 3D viewport on the 2D canvas. The `<scene>` is outside
its own content for transform-context purposes.

Functions compose **left to right** in both contexts: `transform="A B"`
is the matrix product `A · B`, applied to a column vector as
`(A · B) · v`. The leftmost function is outermost; the rightmost
applies first to the local coordinates. This matches SVG 1.1 and CSS.

### 6.2 3D transform functions

Inside `<scene>`, the permitted `transform` functions are:

| Function | Effect |
|---|---|
| `translate(tx, ty)` | translate by `(tx, ty, 0)` |
| `translate3d(tx, ty, tz)` | translate by `(tx, ty, tz)` |
| `scale(s)` | uniform scale by `s` on all three axes |
| `scale(sx, sy)` | scale by `(sx, sy, 1)` |
| `scale3d(sx, sy, sz)` | scale by `(sx, sy, sz)` |
| `rotateX(angle)` | rotate `angle` about the local X axis |
| `rotateY(angle)` | rotate `angle` about the local Y axis |
| `rotateZ(angle)` | rotate `angle` about the local Z axis |
| `rotate(angle)` | alias for `rotateZ(angle)` — preserves SVG 1.1 reading of "rotate" as in-plane |
| `rotate3d(x, y, z, angle)` | rotate `angle` about the axis `(x, y, z)` (axis is normalised internally; the zero vector is an error) |
| `matrix3d(m11, m12, …, m44)` | apply a 4×4 matrix, **column-major** (16 numbers, same convention as CSS Transforms 2) |

The SVG 1.1 2D-only functions `matrix` / `skewX` / `skewY` are **not**
defined inside `<scene>`. Equivalent shears are expressible via
`matrix3d`; a future draft may re-introduce them if a 3D-skew use case
emerges.

### 6.3 Composition and inheritance

Transforms on nested elements compose by matrix product, from ancestor
to descendant. Inside `<scene>`, a primitive's world-space (3D)
transform is:

```
M_world = M_scene · M_group_1 · … · M_group_n · M_self
```

where `M_self` is `transform` on the primitive itself, each
`M_group_i` is `transform` on an ancestor `<g>`, and `M_scene` is
`transform` on the surrounding `<scene>` (identity in v0 if the
`<scene>` has no transform). The `<scene>`'s own placement on the 2D
canvas is governed by its 2D `transform` and its SVG 1.1 viewport
attributes, separately from the 3D composition above.

### 6.4 Reference vs. CSS Transforms

CSS Transforms 2 attaches a `transform-origin` to every transformed
element (default `50% 50% 0` for an HTML element). svg3 v0 fixes the
transform origin at the **local origin** (the primitive's centroid) and
does **not** support a separate `transform-origin` property. Rotating
about a non-origin point is expressible by composing translations
(`translate3d(cx,cy,cz) rotateZ(a) translate3d(-cx,-cy,-cz)`).

---

## 7. Camera and viewport

### 7.1 The 2D viewport (SVG 1.1)

**Status:** \[Roadmap\]

The document's 2D viewport is the `<svg>` root and is governed by SVG
1.1's `width`, `height`, `viewBox`, and `preserveAspectRatio`
attributes (see [SVG 1.1 ch. 7](https://www.w3.org/TR/SVG11/coords.html)).
This is inherited from SVG 1.1 and not re-specified here; it is
Roadmap in the v0 implementation.

### 7.2 The 3D camera

**Status:** \[Out of scope (v0) — renderer-side\]

In v0 the **3D camera is not part of the document**. The renderer host
constructs a [`svg3_render::Camera`](svg3-render/src/lib.rs)
(`eye` / `target` / `fov_y`) and a `RenderConfig` (format + dimensions),
passes them in, and gets a frame. The `<scene>` element carries no
camera attributes. The camera's projected output composites into the
`<scene>` element's 2D rectangle on the canvas.

Rationale: a document that ships its own camera couples scene
authoring to a single framing decision, which doesn't match how 3D
content is consumed (most viewers want to orbit/fly). The
renderer-side camera keeps the document portable. A future draft may
add a `<camera>` element or `default-camera` attribute on `<scene>` as
a hint the host may honour or override.

---

## 8. Styling

**Status:** \[Roadmap\]

### 8.1 CSS via Stylo

svg3 resolves computed styles with [Stylo](https://crates.io/crates/stylo),
Servo's CSS engine. The integration follows
[Blitz (`blitz-dom`)](https://github.com/DioxusLabs/blitz) as the
canonical reference for driving Stylo over a non-browser DOM. svg3
inherits Stylo's behavior wholesale for everything CSS already covers:
cascade order, specificity, inheritance, custom properties, media
queries (when applicable), `!important`, etc. The svg3 spec only defines
**which properties have rendering meaning** ([§9](#9-paint-and-material))
and how presentation attributes map to them. SVG 1.1 elements use the
SVG 1.1 property set (`fill`, `stroke`, …) per SVG 1.1 ch. 11.

### 8.2 Where styles come from

In precedence order (matching SVG 1.1 and CSS):

1. The `style` attribute on the element.
2. CSS rules in author stylesheets (`<style>` element — *roadmap*, not
   parsed in v0).
3. **Presentation attributes** on the element (see [§8.3](#83-presentation-attributes)).
4. Inherited values from ancestors.
5. The property's initial value ([§9](#9-paint-and-material)).

### 8.3 Presentation attributes

A **presentation attribute** is an XML attribute whose name matches a CSS
property and whose value is parsed as the property's CSS value. They
participate in the cascade with the same specificity as SVG 1.1 grants
them: a presentation attribute is treated as an author rule with the
lowest specificity, so any matching CSS selector overrides it. v0
defines presentation attributes on 3D content (inside `<scene>`) for:

- `color`
- `opacity`

SVG 1.1 presentation attributes (`fill`, `stroke`, `stroke-width`, …)
apply to SVG 1.1 elements per SVG 1.1; they are not implemented in v0.

### 8.4 Selectors and matching

Selectors are evaluated against the svg3 element tree exactly as Stylo
evaluates them against an HTML tree. Tag-name selectors match svg3 tag
names (`svg`, `scene`, `cube`, `ellipsoid`, `g`, …); `.foo` matches
elements with a `class` attribute containing `foo`; `#foo` matches
elements with `id="foo"`. Pseudo-classes that depend on browser state
(`:hover`, `:focus`, …) have no defined behavior in v0.

---

## 9. Paint and material

**Status:** \[Roadmap\]

### 9.1 Property set for 3D content (v0)

These properties apply to 3D primitives inside `<scene>`:

| Property | Type | Initial | Inherits | Effect |
|---|---|---|---|---|
| `color` | CSS color (sRGB) | `black` | yes | flat surface colour |
| `opacity` | number `[0, 1]` | `1` | no | per-element alpha; multiplied into ancestor opacity |

CSS color syntax follows [CSS Color 4](https://www.w3.org/TR/css-color-4/):
named colors, `#rgb` / `#rrggbb` / `#rrggbbaa`, `rgb()` / `rgba()`,
`hsl()` / `hsla()`. Values outside `[0,1]` for opacity clamp.

### 9.2 Paint for 2D content (SVG 1.1)

SVG 1.1 elements outside `<scene>` use SVG 1.1's paint model — `fill`,
`stroke`, `stroke-width`, paint servers (`url(#…)` referencing
gradients / patterns), and the rest of [SVG 1.1 ch. 11](https://www.w3.org/TR/SVG11/painting.html).
This surface is inherited by reference and not re-specified here; it
is Roadmap in the v0 implementation.

### 9.3 What 3D content does *not* define

The following are **explicitly deferred** for 3D content. Documents may
carry these attributes for future compatibility, but they have no
effect in v0:

- Lighting (directional / point / area lights, ambient term).
- Material models beyond flat colour (PBR, metallic/roughness, IOR).
- Textures, texture coordinates, normal maps.
- Stroking / outlines on 3D primitives.
- Gradients, patterns, and paint-server references (`url(#…)`) on 3D
  primitives. (For 2D content, these come from SVG 1.1.)
- Shadows, ambient occlusion, post-processing.

### 9.4 Rendering model for 3D content (v0)

Every 3D primitive is rendered as a triangle mesh with each vertex
shaded by the element's computed `color`, modulated by computed
`opacity` and ancestor opacities. There are no lights and no
view-dependent shading in v0; appearance depends only on geometry,
transform, color, and opacity. Backface culling, blending, and depth
test are renderer-side concerns and are not specified by the document.

---

## 10. Relationship to SVG 1.1

### 10.1 svg3 is a superset of SVG 1.1

svg3 adopts SVG 1.1 wholesale and adds 3D content. By design:

- The root element is `<svg>` (SVG 1.1 ch. 5).
- SVG 1.1 elements, attributes, and properties retain their SVG 1.1
  semantics by reference; this spec does not re-define them.
- svg3 adds the `<scene>` 3D-context element and the 3D primitives /
  transforms inside it ([§4](#4-elements)–[§7](#7-camera-and-viewport)).
- A document with only SVG 1.1 markup (no `<scene>`) is a valid svg3
  document and a valid SVG 1.1 document — bidirectional compatibility.
- A document with `<scene>` renders correctly in svg3; in a 2D-only SVG
  1.1 renderer, `<scene>` is treated as an unknown element and skipped,
  leaving surrounding 2D content visible.

### 10.2 What svg3 adds to SVG 1.1

| Addition | Effect | Section |
|---|---|---|
| `<scene>` element | Introduces a 3D rendering context as a child of `<svg>`. | [§4.3](#43-scene) |
| `<cube>`, `<ellipsoid>` | 3D primitives, valid inside `<scene>`. | [§4.5](#45-cube), [§4.6](#46-ellipsoid) |
| 3D coordinate system | Right-handed, +Y up, inside `<scene>`. | [§5](#5-coordinate-system-and-units) |
| 3D `transform` functions | `translate3d`, `rotateX/Y/Z`, `rotate3d`, `scale3d`, `matrix3d` — inside `<scene>`. | [§6.2](#62-3d-transform-functions) |
| Host-supplied 3D camera | The renderer (not the document) sets eye/target/fov_y for 3D content. | [§7.2](#72-the-3d-camera) |

### 10.3 SVG 1.1 implementation status

svg3 inherits the entirety of SVG 1.1 by reference. The v0
implementation does not yet cover most of it — the table below tracks
status by SVG 1.1 chapter, **not** a "drop list":

| SVG 1.1 chapter | Status | Notes |
|---|---|---|
| Ch. 3 Rendering model | Roadmap | Painter's algorithm; 3D content is rasterised into its `<scene>` rectangle and then composites. |
| Ch. 5 Document structure | Partial | `<g>` parsed in v0; `<svg>` root parsing tracked in §2; `<defs>`, `<symbol>`, `<use>` Roadmap. |
| Ch. 6 Styling | Partial | Stylo cascade is the wiring; see [§8](#8-styling). |
| Ch. 7 Coordinates / 2D `transform` | Roadmap | 2D `transform` outside `<scene>`. |
| Ch. 8 Paths | Roadmap | `<path>` not parsed in v0. |
| Ch. 9 Basic shapes | Roadmap | `<rect>`, `<circle>`, `<ellipse>`, `<line>`, `<polyline>`, `<polygon>`. |
| Ch. 10 Text + Ch. 20 Fonts | Roadmap (later) | No text in v0; significant scope. |
| Ch. 11 Painting (`fill`, `stroke`) | Roadmap | Applies to SVG 1.1 elements; 3D primitives use `color`/`opacity` ([§9.1](#91-property-set-for-3d-content-v0)). |
| Ch. 12 Color | Inherited | CSS Color 4 supersedes; see [§9](#9-paint-and-material). |
| Ch. 13 Gradients, patterns | Roadmap (later) | |
| Ch. 14 Clipping, masking, compositing | Roadmap (later) | |
| Ch. 15 Filter effects | Out of scope (v0) | May return. |
| Ch. 16 Interactivity, Ch. 17 Linking | Out of scope (v0) | Native-only; no event/scripting model. |
| Ch. 18 Scripting | Out of scope | Native-only, no JS engine. |
| Ch. 19 Animation (SMIL) | Out of scope (v0) | No time model in v0 — see [§11](#11-open-questions-and-future-drafts). |
| Ch. 21 Metadata, Ch. 22 Backwards compat | Inherited | No svg3-specific changes. |
| Ch. 23 Extensibility (`<foreignObject>`) | Roadmap (later) | `<scene>` is itself an extension in the spirit of ch. 23. |

"Inherited" means SVG 1.1's text applies unchanged. "Roadmap" means
svg3 will support it; v0 does not. "Out of scope (v0)" means
explicitly deferred, possibly forever.

---

## 11. Open questions and future drafts

Recorded here so they don't get lost between drafts, with the section
they belong to:

1. **Parser root mismatch (Draft 0 → Draft 1).** `svg3-dom` currently
   enforces `<scene>` as root; this spec says `<svg>`. A follow-up PR
   ([§2](#2-status-and-conformance)) flips this and adds a parse test
   for an `<svg>`-rooted document containing `<scene>`.
2. **SVG 1.1 element coverage order** — likely basic shapes first, then
   paths, then text — [§10.3](#103-svg-11-implementation-status).
3. **Nested `<scene>`** — flatten, sub-context, or error? Undefined in
   v0 — [§4.3](#43-scene).
4. **`<scene>` placement on the canvas** — `x` / `y` / `width` / `height`
   attributes (matching nested `<svg>` / `<foreignObject>`) vs. always
   inheriting the parent's box — [§4.3](#43-scene).
5. **More 3D primitives** (`<cylinder>`, `<plane>`, `<mesh>` with vertex
   data) — [§4](#4-elements).
6. **3D camera in the document** — `<camera>` element vs. attributes on
   `<scene>`, multiple cameras (named views) — [§7.2](#72-the-3d-camera).
7. **Lighting and a real material model** — directional light + diffuse
   term as the minimum viable set, or jump to PBR — [§9](#9-paint-and-material).
8. **External `<style>` parsing** — the cascade exists in Stylo; only
   the wiring is missing — [§8.2](#82-where-styles-come-from).
9. **Animation** — no time model in v0. Whether to grow one (CSS
   Animations? SMIL? a discrete keyframe element?) is open.
10. **Error recovery** — the parser is currently fail-fast; SVG 1.1 is
    not. Conformance ([§2](#2-status-and-conformance)) will need a
    decision here.

Each item is a roadmap candidate, not a commitment. Per the project's
no-speculative-scaffolding rule (see [AGENTS.md](AGENTS.md)), they will
not appear in code or in the normative parts of this spec until a
milestone needs them.
