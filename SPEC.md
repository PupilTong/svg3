# svg3 Specification — Draft 0

**Status:** working draft. The svg3 implementation is at an early scaffold (see
[README.md](README.md) roadmap). This document is **descriptive of intent and
normative for what already ships**, marked per section. It will firm up as the
pipeline (parse → style → render) reaches feature parity with the spec.

**Scope:** the on-the-wire document language and the rendering model. Public
Rust API surface, build/run instructions, and contributor conventions live in
[README.md](README.md) and [AGENTS.md](AGENTS.md).

This spec is layered on top of SVG 1.1 — see
[§10. Relationship to SVG 1.1](#10-relationship-to-svg-11) for the
inherit / change / drop table. Where svg3 borrows from existing web specs, the
canonical references are:

- [SVG 1.1 (W3C)](https://www.w3.org/TR/SVG11/) — XML structure, presentation
  attribute model, cascade hookup.
- [CSS Transforms Module Level 2](https://www.w3.org/TR/css-transforms-2/) —
  3D transform function syntax and composition rules.
- [CSS Color Module Level 4](https://www.w3.org/TR/css-color-4/) — paint
  values.
- The host CSS engine for everything else: svg3 resolves styles through
  [Stylo](https://crates.io/crates/stylo) and inherits its CSS semantics
  directly (see [§8. Styling](#8-styling)).

---

## 1. Introduction

svg3 is an **XML-based document language for three-dimensional scenes**.
It keeps SVG's authoring shape — declarative element tree, presentation
attributes, CSS styling — and replaces the 2D drawing model with 3D
primitives, a 3D coordinate system, and 3D transforms.

A document describes geometry only. The host application supplies the
camera, the framebuffer, and the rendering surface (see
[§7. Camera and viewport](#7-camera-and-viewport)); the document neither
embeds a viewport nor schedules animation in v0.

### 1.1 Why a 3D extension of SVG

SVG already provides the things a scene description needs: an XML tree, a
mature attribute/styling vocabulary, CSS cascade semantics, and an
authoring ergonomics that designers know. The 2D-only parts (paths, text,
filters, gradients) are dropped here so a small, focused spec can describe
3D primitives styled the same way.

svg3 is **not** an extension submitted to the SVG WG, and it is not
intended to render inside a browser. It is a native, document-driven 3D
renderer; the syntactic resemblance to SVG is deliberate so that authoring
intuition transfers.

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

There is no conformance suite yet. When v0.1 of the implementation ships
(roadmap items 3–4 in [README.md](README.md): mesh generation, render a
single cube, real Stylo cascade), conformance will be defined as:

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
typed errors in `svg3-dom`; the reference parser is fail-fast.

---

## 3. Document syntax

**Status:** \[Implemented\]

### 3.1 Wire format

A svg3 document is an XML 1.0 text document. The reference parser is
[`svg3_dom::parse`](svg3-dom/src/lib.rs) (built on
[`quick-xml`](https://crates.io/crates/quick-xml)). The following XML
constructs are recognised:

| Construct | Handling |
|---|---|
| Start tag (`<scene>`) | begins a new element |
| Empty-element tag (`<cube/>`) | begins and ends an element |
| End tag (`</scene>`) | closes the current element |
| Attributes (`name="value"`) | stored verbatim after XML-unescape |
| Text, CDATA, comments, PIs, DOCTYPE, XML decl | **ignored** in v0 |
| Namespaces | **not handled** in v0 (prefixes are part of the tag name) |

The mandatory root element is `<scene>`. A document with any other root
is rejected with `ParseError::UnexpectedRoot`.

### 3.2 Attribute values

Attribute values are stored as raw `String`s on the owning element. The
parser does **not** interpret them into typed forms; that responsibility
sits with the layer that consumes the value (the style cascade for
`class`/`id`/`style`, the renderer for geometry attributes like `size`).

Rationale: the same string needs to flow into Stylo (which expects raw
attribute text for selector matching) and into the renderer (which parses
its own typed form). One canonical representation, parsed at the point of
use, avoids double-conversion. The reference DOM has shipped exactly this
shape since [`svg3-dom#4`](https://github.com/anthropics/apps/pull/4).

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

Not yet registered. Files are conventionally suffixed `.svg3` (this is
descriptive, not normative).

---

## 4. Elements

### 4.1 Element list (v0)

| Tag | Kind | Status | Section |
|---|---|---|---|
| `<scene>` | root container | implemented (parse only) | [§4.2](#42-scene) |
| `<g>`, `<group>` | grouping / transform | implemented (parse only) | [§4.3](#43-g--group) |
| `<cube>` | axis-aligned box primitive | implemented (parse only) | [§4.4](#44-cube) |
| `<ellipsoid>` | ellipsoid primitive | implemented (parse only) | [§4.5](#45-ellipsoid) |

All other tags parse as `ElementKind::Unknown` (preserving the tag name)
so non-svg3 markup round-trips through the DOM. Unknown tags are
**not** rendered.

Attributes named in this section are the v0 set. Any other attribute is
preserved on the element (for the cascade's benefit — `id`, `class`,
`style`, and any author-defined attribute can flow through), but only
the listed attributes affect rendering.

### 4.2 `<scene>`

**Status:** \[Partial\]

The document root. Acts as a grouping element for its children; does not
itself render. Attributes affecting rendering: none in v0. (Future: a
default-camera element / attribute is on the roadmap — see
[§7. Camera and viewport](#7-camera-and-viewport).)

### 4.3 `<g>` / `<group>`

**Status:** \[Implemented (parse only)\]

Grouping element. Equivalent to SVG 1.1's `<g>`. Both spellings are
accepted; the canonical tag is `<g>`. Permitted attributes:

- `transform` — see [§6. Transforms](#6-transforms).
- `id`, `class`, `style` — see [§8. Styling](#8-styling).

A `<g>` is a transform/style boundary; it has no intrinsic geometry.

### 4.4 `<cube>`

**Status:** \[Roadmap (geometry)\]

An axis-aligned box centered at the local origin.

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

### 4.5 `<ellipsoid>`

**Status:** \[Roadmap (geometry)\]

An ellipsoid centered at the local origin.

| Attribute | Type | Default | Meaning |
|---|---|---|---|
| `r` | length | `1` | uniform radius (sets `rx`/`ry`/`rz`) |
| `rx` | length | from `r` | radius along local X |
| `ry` | length | from `r` | radius along local Y |
| `rz` | length | from `r` | radius along local Z |
| `transform` | transform-list | identity | see [§6](#6-transforms) |
| `id`, `class`, `style` | — | — | see [§8](#8-styling) |

If `r` and any of `rx`/`ry`/`rz` are both present, the per-axis attribute
wins. A `<ellipsoid>` with `rx=ry=rz` is a sphere.

---

## 5. Coordinate system and units

**Status:** \[Roadmap\]

### 5.1 Spaces

- **Object space.** Each primitive is defined relative to a local origin
  (the centroid for `<cube>` and `<ellipsoid>`).
- **Local space.** Object space with the element's `transform` applied.
- **World space.** Local space with all ancestor `transform`s applied,
  composing from root to leaf.
- **View space, clip space.** Owned by the camera (see
  [§7](#7-camera-and-viewport)); not addressable from the document.

### 5.2 Handedness and axes

svg3 is **right-handed** with:

- **+X** to the right,
- **+Y** up,
- **+Z** toward the viewer (out of the screen).

This matches the convention in [`svg3_render::Camera`](svg3-render/src/lib.rs)
(`glam::Mat4::look_at_rh` with `Vec3::Y` as up) and the prevailing
convention in 3D graphics tooling.

This is a **deliberate divergence from SVG 1.1**, which uses a
left-handed-ish 2D system with +Y down. The visual-debugging cost of
keeping SVG's +Y-down in a 3D scene (every rotation reading mirrored)
outweighs the consistency benefit.

### 5.3 Units

A "user unit" is the base length. There are no `px`/`em`/`pt` qualifiers
in v0 — `size="2"` is interpreted as `2` user units, full stop. The
camera projection and the framebuffer resolution determine how many
pixels a user unit occupies on screen.

Lengths are parsed as IEEE-754 `f32`. Negative lengths are not allowed
for radii or extents (`size`, `r`, `width`, `height`, `depth`, `rx`,
`ry`, `rz`).

CSS-style length units (`%`, `em`, `vh`, …) are **out of scope for v0**.

### 5.4 Angles

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

Permitted on `<g>`, `<cube>`, `<ellipsoid>` (and on any future primitive).
The value is a space- or whitespace-separated list of **transform
functions**. Functions compose **left to right**: `transform="A B"` is
the matrix product `A · B`, applied to a column vector as `(A · B) · v`.
The leftmost function is the outermost transform; equivalently, the
rightmost is applied first to the local coordinates. This matches both
SVG 1.1 and [CSS Transforms 2](https://www.w3.org/TR/css-transforms-2/).

### 6.2 Transform functions

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

The 2D-only SVG 1.1 functions `skewX` / `skewY` / `matrix` are **not**
defined in v0. They may return in a future draft if a 3D-skew use case
emerges; in the meantime, equivalent shears are expressible via
`matrix3d`.

### 6.3 Composition and inheritance

Transforms on nested elements compose by matrix product, from root to
leaf. Conceptually, a primitive's world-space transform is:

```
M_world = M_scene · M_group_1 · … · M_group_n · M_self
```

where `M_self` is `transform` on the primitive itself and each
`M_group_i` is `transform` on an ancestor `<g>`. `M_scene` is identity
in v0 (the `<scene>` carries no transform attribute).

### 6.4 Reference vs. CSS Transforms

CSS Transforms 2 attaches a `transform-origin` to every transformed
element (default `50% 50% 0` for an HTML element). svg3 v0 fixes the
transform origin at the **local origin** (the primitive's centroid) and
does **not** support a separate `transform-origin` property. Rotating
about a non-origin point is expressible by composing translations
(`translate3d(cx,cy,cz) rotateZ(a) translate3d(-cx,-cy,-cz)`).

---

## 7. Camera and viewport

**Status:** \[Out of scope (v0)\]

In v0 the camera is **not** part of the document. The renderer host
constructs a [`svg3_render::Camera`](svg3-render/src/lib.rs)
(`eye`/`target`/`fov_y`) and a `RenderConfig` (format + dimensions),
passes them in, and gets a frame. The document carries no `viewBox`
analog and no `<camera>` element.

Rationale: a document that ships its own camera couples scene authoring
to a single framing decision, which doesn't match how 3D content is
consumed (most viewers want to orbit/fly). The renderer-side camera
keeps the document portable. A future draft may add a `<camera>` element
or `default-camera` attribute on `<scene>` as a *hint* the host may
honour or override.

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
and how presentation attributes map to them.

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
defines presentation attributes for:

- `color`
- `opacity`

Other property names appearing as attributes are stored on the element
but have no rendering effect in v0.

### 8.4 Selectors and matching

Selectors are evaluated against the svg3 element tree exactly as Stylo
evaluates them against an HTML tree. Tag-name selectors match svg3 tag
names (`cube`, `ellipsoid`, `g`, `scene`); `.foo` matches elements with a
`class` attribute containing `foo`; `#foo` matches elements with `id="foo"`.
Pseudo-classes that depend on browser state (`:hover`, `:focus`, …) have
no defined behavior in v0.

---

## 9. Paint and material

**Status:** \[Roadmap\]

### 9.1 Property set (v0)

| Property | Type | Initial | Inherits | Effect |
|---|---|---|---|---|
| `color` | CSS color (sRGB) | `black` | yes | flat surface colour |
| `opacity` | number `[0, 1]` | `1` | no | per-element alpha; multiplied into ancestor opacity |

CSS color syntax follows [CSS Color 4](https://www.w3.org/TR/css-color-4/):
named colors, `#rgb` / `#rrggbb` / `#rrggbbaa`, `rgb()` / `rgba()`,
`hsl()` / `hsla()`. Values outside `[0,1]` for opacity clamp.

### 9.2 What v0 does *not* define

The following are **explicitly deferred**. Documents may carry these
attributes for future compatibility, but they have no effect in v0:

- Lighting (directional / point / area lights, ambient term).
- Material models beyond flat colour (PBR, metallic/roughness, IOR).
- Textures, texture coordinates, normal maps.
- Stroking / outlines (no 3D analog of SVG 1.1's `stroke` is defined).
- Gradients, patterns, and any paint-server reference (`url(#…)`).
- Shadows, ambient occlusion, post-processing.

### 9.3 Rendering model (v0)

Every drawable primitive is rendered as a triangle mesh with each vertex
shaded by the element's computed `color`, modulated by computed
`opacity` and ancestor opacities. There are no lights and no view-
dependent shading in v0; the appearance depends only on geometry,
transform, color, and opacity. Backface culling, blending, and depth
test are renderer-side concerns and are not specified by the document.

---

## 10. Relationship to SVG 1.1

### 10.1 What svg3 inherits

- XML 1.0 syntax (parsed by quick-xml; see [§3](#3-document-syntax)).
- The presentation-attribute / style-attribute / external-stylesheet
  layering model ([§8](#8-styling)).
- CSS cascade and inheritance — by way of Stylo, which is also the
  cascade engine Firefox/Servo use for HTML and SVG.
- The `<g>` grouping element and `transform` attribute semantics
  (extended to 3D — [§6](#6-transforms)).
- The convention of `id` / `class` / `style` attributes for hooking up
  CSS.

### 10.2 What svg3 changes

| SVG 1.1 | svg3 | Notes |
|---|---|---|
| `<svg>` | `<scene>` | The roots differ; svg3 will not parse an `<svg>`-rooted document. |
| 2D `transform` (`matrix`, `skewX/Y`) | 3D `transform` (matching [CSS Transforms 2](https://www.w3.org/TR/css-transforms-2/)) | See [§6.2](#62-transform-functions); SVG-1.1-only functions are unsupported in v0. |
| `viewBox` + `preserveAspectRatio` | host-side camera | The document carries no viewport ([§7](#7-camera-and-viewport)). |
| +Y down | +Y up, right-handed | Deliberate divergence ([§5.2](#52-handedness-and-axes)). |
| `fill`, `stroke` | `color` (only) | No stroke model in v0; `fill` is not recognised. |
| `<circle>` / `<ellipse>` | `<ellipsoid>` | 3D generalisation. |
| `<rect>` | (none — use `<cube>`) | No 2D rectangle primitive. |

### 10.3 What svg3 drops

These SVG 1.1 features have **no** v0 equivalent and are out of scope as
of this draft:

- All `<path>` / `d` geometry.
- `<text>`, `<tspan>`, `<textPath>`, fonts ([SVG 1.1 ch. 10, 20](https://www.w3.org/TR/SVG11/text.html)).
- Filter effects ([ch. 15](https://www.w3.org/TR/SVG11/filters.html)).
- Gradients, patterns, markers, paint servers ([ch. 13](https://www.w3.org/TR/SVG11/pservers.html), [ch. 11](https://www.w3.org/TR/SVG11/painting.html#Markers)).
- Clipping, masking, compositing ([ch. 14](https://www.w3.org/TR/SVG11/masking.html)).
- Declarative animation / SMIL ([ch. 19](https://www.w3.org/TR/SVG11/animate.html)) — svg3 has no time model.
- Scripting ([ch. 18](https://www.w3.org/TR/SVG11/script.html)) — native-only, no JS engine.
- Linking, `<a>` ([ch. 17](https://www.w3.org/TR/SVG11/linking.html)).
- `<defs>` / `<use>` / `<symbol>` reuse machinery — *roadmap*, not v0.
- `<foreignObject>` ([ch. 23](https://www.w3.org/TR/SVG11/extend.html)).
- `<switch>` + `requiredFeatures` / `requiredExtensions` / `systemLanguage`.
- Conditional processing and accessibility (`<title>`, `<desc>`) — may
  return.

---

## 11. Open questions and future drafts

Recorded here so they don't get lost between drafts, with the section
they belong to:

1. **More primitives** (`<cylinder>`, `<plane>`, `<mesh>` with vertex
   data) — [§4](#4-elements).
2. **Camera in the document** — `<camera>` element vs. attributes on
   `<scene>`, and whether multiple cameras (named views) are worth the
   complexity — [§7](#7-camera-and-viewport).
3. **Lighting and a real material model** — minimum viable set
   (one directional light + a diffuse term) vs. jumping to PBR — [§9](#9-paint-and-material).
4. **`<defs>` / `<use>` for primitive reuse** — same shape as SVG 1.1, or
   a leaner pure-reference form — [§10.3](#103-what-svg3-drops).
5. **External `<style>` parsing** — the cascade exists in Stylo; only
   the wiring is missing — [§8.2](#82-where-styles-come-from).
6. **Animation** — no time model in v0. Whether to grow one (CSS
   Animations? a discrete keyframe element?) is open — [§10.3](#103-what-svg3-drops).
7. **Error recovery** — the parser is currently fail-fast; SVG 1.1 is
   not. Conformance ([§2](#2-status-and-conformance)) will need
   a decision here.

Each item is a roadmap candidate, not a commitment. Per the project's
no-speculative-scaffolding rule (see [AGENTS.md](AGENTS.md)), they will
not appear in code or in the normative parts of this spec until a
milestone needs them.
