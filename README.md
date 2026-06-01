<h1 align="center">svg3</h1>

<p align="center"><b>An extended SVG renderer for 3D models — on the GPU.</b></p>

<p align="center">
  <a href="https://github.com/PupilTong/svg3/actions/workflows/ci.yml"><img src="https://github.com/PupilTong/svg3/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MPL--2.0-brightgreen.svg" alt="License: MPL-2.0"></a>
  <img src="https://img.shields.io/badge/rust-nightly-orange.svg" alt="Rust nightly">
  <img src="https://img.shields.io/badge/GPU-wgpu-blueviolet.svg" alt="wgpu">
</p>

<p align="center">
  <img src="docs/cube.png" alt="A textured 3D cube rendered by svg3" width="360">
  &nbsp;
  <img src="docs/shapes.png" alt="2D gradients and opacity rendered by svg3" width="360">
</p>
<p align="center"><sub>Rendered headlessly by the <code>svg3</code> CLI from the documents in <a href="svg3-skill/examples/"><code>svg3-skill/examples/</code></a>.</sub></p>

---

## What is svg3?

`svg3` is an **opt-in extension to SVG 1.1**. A document is ordinary SVG until
its root `<svg>` carries `extension="pupiltong"`; with that attribute it may
also use three-dimensional graphics elements (`<cube>`, `<ellipsoid>`,
`<cylinder>`, `<surface>`) and 3D transform functions, all rasterised on the GPU
with [wgpu](https://crates.io/crates/wgpu). Without it, the renderer behaves as
a normal SVG renderer and ignores the 3D features.

If you have never written SVG, start with the [60-second SVG
primer](#a-60-second-svg-primer); if you know SVG and just want the 3D bits, jump
to [the 3D extension](#the-3d-extension). The formal grammar lives in
[SPEC.md](SPEC.md) (editor's draft); this README is the practical, example-driven
guide to what the renderer actually supports today.

> **Status: early but usable.** The renderer covers a growing subset of SVG 1.1
> plus the svg3 3D primitives — enough to author real images, as the gallery
> above shows. The gaps are real and called out throughout, and collected under
> [Limitations & gotchas](#limitations--gotchas). A full web-platform CSS cascade
> (the historical [Stylo](https://crates.io/crates/stylo) direction) is a paused
> roadmap item; styling today reads **presentation attributes** (e.g.
> `fill="red"`), not the CSS `style="…"` attribute (with two narrow exceptions —
> see [Colors & paint](#colors--paint)).

## Contents

- [Install](#install)
- [Render your first image](#render-your-first-image)
- [A 60-second SVG primer](#a-60-second-svg-primer) — for developers new to SVG
- [The 3D extension](#the-3d-extension) — the part of svg3 that is *not* standard SVG
- [Element & attribute reference](#element--attribute-reference)
  - [Colors & paint](#colors--paint) · [Lengths & units](#lengths--units) ·
    [2D shapes](#2d-shapes) · [Stroke styling](#stroke-styling) ·
    [Gradients](#gradients) · [Patterns](#patterns) · [Markers](#markers) ·
    [Filters](#filters) · [Clipping & masking](#clipping--masking) ·
    [Nested viewports](#nested-svg-viewports) ·
    [3D primitives](#3d-primitives) · [3D transforms](#3d-transform-functions) ·
    [`<svg>` as a 3D texture](#an-svg-as-a-3d-texture)
- [Limitations & gotchas](#limitations--gotchas)
- [CLI reference](#cli-reference)
- [Generate images from your AI coding agent](#generate-images-from-your-ai-coding-agent)
- [Interactive macOS app](#interactive-macos-app)
- [How it works](#how-it-works) · [Workspace](#workspace) · [Build & run](#build--run) · [Roadmap](#roadmap) · [Contributing](#contributing) · [License](#license)

## Install

You need a **Rust nightly** toolchain (pinned for you — see [Build &
run](#build--run)) and a **GPU adapter** (svg3 rasterises on the GPU via wgpu;
on a typical desktop or laptop this is automatic).

```sh
# Build & install the headless renderer (a binary named `svg3`) from a checkout:
cargo install --path svg3-cli
svg3 --version            # confirm it's on your PATH

# …or run it without installing, straight from the workspace:
cargo run -p svg3-cli -- --help
```

No GPU on this machine (e.g. a CI box)? You can still parse and validate a
document with [`--check`](#cli-reference) — it needs no GPU.

## Render your first image

The `svg3` command turns a document (a file, or stdin) into a PNG. The whole
workflow is a tight loop: **author a document → render it → look at the PNG →
iterate.**

```sh
# A bundled 2D demo — gradients and opacity:
svg3 svg3-skill/examples/shapes.svg -o shapes.png

# A bundled 3D demo — a textured cube. `--camera` adds a perspective view:
svg3 svg3-skill/examples/cube.svg3 -o cube.png --camera
```

Now author one yourself. Save this as `hello.svg` and render it:

```xml
<svg xmlns="http://www.w3.org/2000/svg" width="256" height="256">
  <rect width="256" height="256" fill="#0a1020"/>
  <circle cx="128" cy="128" r="80" fill="#ff7a59"/>
</svg>
```

```sh
svg3 hello.svg            # writes hello.png (256×256)
```

Then open `hello.png` and look at it. A zero exit code only means a file was
written — it does **not** tell you the picture is what you intended. Edit,
re-render, repeat.

**File-extension convention.** Save plain SVG as `.svg` and documents that opt
into the 3D extension (`extension="pupiltong"`) as `.svg3`. The renderer keys off
the root attribute, not the file name, so this is just a convention — but a
helpful one.

## A 60-second SVG primer

*Skip this if you already write SVG.* SVG is an XML format: you describe shapes
declaratively and the renderer paints them.

**The document skeleton.** Everything lives inside a root `<svg>` with a pixel
size:

```xml
<svg xmlns="http://www.w3.org/2000/svg" width="400" height="300">
  <!-- shapes go here -->
</svg>
```

**The coordinate system.** The origin `(0, 0)` is the **top-left** corner. X
grows to the **right**, Y grows **down**. One user unit = one output pixel — so
in a 400×300 document, `x="400"` is the right edge and `y="300"` is the bottom.
(svg3 does not scale the root by `viewBox` yet; see
[Limitations](#limitations--gotchas).)

**Shapes are painted in document order** — later elements draw on top of earlier
ones (the "painter's algorithm"). So a background `<rect>` comes first:

```xml
<svg xmlns="http://www.w3.org/2000/svg" width="400" height="300">
  <rect width="400" height="300" fill="#1b2a4a"/>            <!-- backdrop -->
  <circle cx="160" cy="150" r="90" fill="#ffd9a0"/>          <!-- behind   -->
  <circle cx="240" cy="150" r="90" fill="#ff7a59" opacity="0.8"/> <!-- in front -->
  <rect x="40" y="40" width="120" height="60" rx="12"
        fill="none" stroke="#7ad7f0" stroke-width="6"/>      <!-- outlined  -->
</svg>
```

That is most of what you need: pick a shape, give it geometry attributes, and
paint it with `fill` (interior) and/or `stroke` (outline). The full set of
shapes, paints, and effects is in the [reference](#element--attribute-reference)
below. Two svg3-specific things to internalise now:

- **Colors** are hex (`#rrggbb` / `#rgb`) or one of ~20 named colors. `rgb(…)`,
  `hsl(…)`, and most CSS color names are **not** supported — see
  [Colors & paint](#colors--paint).
- **Style goes in attributes, not CSS.** Write `fill="red"`, never
  `style="fill:red"`. The `style` attribute is mostly ignored.

## The 3D extension

This is the part of svg3 that is **not** standard SVG — so don't assume
familiarity from elsewhere. It is small and learnable in one sitting.

### 1. Opt in on the root

3D elements and 3D transforms do **nothing** unless the root `<svg>` carries
`extension="pupiltong"`. Without it, `<cube>` and friends are unknown elements
and render as empty (exactly how a normal SVG viewer would treat them).

```xml
<svg xmlns="http://www.w3.org/2000/svg" extension="pupiltong" width="512" height="512">
  <cube cx="256" cy="256" cz="0" size="200" fill="#e8743b"/>
</svg>
```

### 2. The coordinate system gains a Z axis

2D content stays exactly where it was — in the plane `z = 0`. svg3 adds a third
axis: **+X right, +Y down, +Z toward the viewer** (a left-handed system). The Z
axis is addressed by the `cz` / `cz`-like attributes on 3D elements and by the
3D transform functions. Greater Z is *closer to you*.

### 3. The four 3D primitives

Each is centred at `(cx, cy, cz)` and painted with `fill` and `opacity` (no
`stroke` on 3D elements). Minimal, render-ready examples (all assume
`extension="pupiltong"` on the root):

```xml
<!-- A cube. `size` sets all three edges; or set width/height/depth separately. -->
<cube cx="256" cy="256" cz="0" size="200" fill="#f2c14e"/>

<!-- An ellipsoid (a sphere when rx=ry=rz). `r` sets all three radii. -->
<ellipsoid cx="256" cy="256" cz="0" r="120" fill="#10b981"/>

<!-- A cylinder, axis along Z. `depth` is its length; `r` the cap radius. -->
<cylinder cx="256" cy="256" cz="0" r="100" depth="160" fill="#c14b2b"/>

<!-- A surface: child <path> cross-sections linked by Bezier patches.
     `d="M 0 L 1"` rules a flat strip between path 0 and path 1. -->
<surface d="M 0 L 1" fill="#ec4899">
  <path d="M 180 200 L 332 200"/>
  <path d="M 180 200 L 332 200" transform="translate(0, 120)"/>
</surface>
```

See [3D primitives](#3d-primitives) for every attribute.

### 4. Transforms — and the one gotcha

You position and orient 3D content with the `transform` attribute, mixing SVG
1.1 functions (`translate`, `scale`, `rotate`, …) with svg3's 3D functions
(`rotateX`, `rotateY`, `rotateZ`, `rotate3d`, `translate3d`, `translateZ`,
`scale3d`, `scaleZ`, `matrix3d`).

**The gotcha:** like SVG 1.1, transforms rotate about the coordinate **origin**,
not the shape's centre. To spin a primitive in place, build it at the origin
(`cx=cy=cz=0`) and *then* translate it into the frame. Functions compose
left-to-right:

```xml
<!-- Build at origin → rotate about origin → move to the centre of a 512² frame: -->
<cube cx="0" cy="0" cz="0" size="200"
      transform="translate(256 256) rotateY(35deg) rotateX(-22deg)"
      fill="#e8743b"/>
```

Angles take `deg` (default), `rad`, `grad`, or `turn`. Full list:
[3D transforms](#3d-transform-functions).

### 5. The camera: orthographic by default, perspective on demand

The viewing camera is supplied by the renderer — it is **not** addressable from
the document (no camera element/attribute exists). The CLI gives you two:

- **Default (no flag):** an **orthographic** (parallel) projection. 3D depth
  still resolves — nearer Z occludes farther Z — but there is no perspective
  foreshortening. Good for diagrams and isometric-style looks.
- **`--camera`:** a **perspective** camera that frames the `width × height`
  region head-on. Center 3D content at `(width/2, height/2)`. This is what you
  want for a "photographed object" look.

```sh
svg3 scene.svg3 -o flat.png            # orthographic
svg3 scene.svg3 -o deep.png --camera   # perspective
```

### 6. Depth compositing with 2D content

3D primitives depth-test against each other and against 2D content by Z: a
`<cube>` at `cz="20"` (toward you) draws in front of a `<rect>` in the `z = 0`
plane; at `cz="-20"` it draws behind it. One caveat: a *full-frame 2D backdrop*
and 3D primitives don't always composite cleanly, so for 3D scenes prefer a
transparent background (or a backdrop placed clearly behind, at negative Z).

### 7. Texturing a primitive with a nested `<svg>`

A primitive's `fill` can reference a nested `<svg>` as a **paint server** — its
rasterised content is wrapped over the 3D surface as a texture. This is how the
hero cube is painted. See [An `<svg>` as a 3D texture](#an-svg-as-a-3d-texture).

---

## Element & attribute reference

This section documents what the **renderer actually consumes today**. Anything
not listed here is either ignored or rendered as empty, even if it is valid SVG.
The formal language (including 3D elements not yet fully implemented) is in
[SPEC.md](SPEC.md).

### Colors & paint

| Property | Applies to | Values | Default |
|---|---|---|---|
| `fill` | shapes, 3D primitives | a [color](#supported-colors), `url(#id)` ([gradient](#gradients)/[pattern](#patterns)/[texture](#an-svg-as-a-3d-texture)), or `none` | `black` |
| `stroke` | 2D shapes (not 3D) | a [color](#supported-colors) or `none` | `none` |
| `fill-opacity` | shapes | `0`–`1` or a percentage | `1` |
| `stroke-opacity` | 2D shapes | `0`–`1` or a percentage | `1` |
| `opacity` | shapes | `0`–`1` or a percentage (multiplies fill *and* stroke) | `1` |

#### Supported colors

Colors are **hex or a small named set only**:

- `#rgb` and `#rrggbb` hex (case-insensitive).
- These named colors: `black`, `white`, `red`, `green`, `blue`, `yellow`,
  `cyan` / `aqua`, `magenta` / `fuchsia`, `gray` / `grey`, `silver`, `maroon`,
  `navy`, `orange`, `purple`, `lime`, `teal`.

**Not supported:** `rgb(…)` / `rgba(…)`, `hsl(…)`, 8-digit `#rrggbbaa`, and the
hundreds of other CSS named colors (e.g. `skyblue`, `crimson`). An unrecognised
`fill` value falls back to black; an unrecognised `stroke` value draws no stroke.
Need a specific color? Convert it to hex. For partial transparency, use the
opacity properties (or an `#rrggbb` plus `fill-opacity`).

> **Presentation attributes, not `style`.** Set paint and geometry as XML
> attributes (`fill="#ff0000"`), **not** via the CSS `style` attribute. The
> renderer consults `style="…"` in only two places: `stop-color` /
> `stop-opacity` / `offset` on a `<stop>`, and `overflow` on a nested `<svg>`.
> Everywhere else, `style="fill:red"` is ignored.

### Lengths & units

Lengths are a plain number (user units = pixels), a `px`-suffixed number
(`12px`, identical to `12`), or a percentage (`50%`, resolved against the
relevant viewport extent). Other CSS units — `em`, `rem`, `pt`, `cm`, `in` — are
**not** supported.

### 2D shapes

All accept `fill`, `stroke`, the [stroke styling](#stroke-styling) properties,
opacity, `transform`, `clip-path`, `mask`, and `filter`.

| Element | Geometry attributes |
|---|---|
| `<rect>` | `x`, `y`, `width`, `height`, `rx`, `ry` (rounded corners) |
| `<circle>` | `cx`, `cy`, `r` |
| `<ellipse>` | `cx`, `cy`, `rx`, `ry` |
| `<line>` | `x1`, `y1`, `x2`, `y2` (stroke only — no fill) |
| `<polyline>` | `points` (an open run of `x,y` pairs) |
| `<polygon>` | `points` (auto-closed; fills concave outlines correctly) |
| `<path>` | `d` (the SVG path mini-language: `M L H V C S Q T A Z`), `fill-rule` = `nonzero` (default) or `evenodd` |
| `<g>` | grouping container — applies its `transform` to children |
| `<defs>` | holds referenced definitions (gradients, patterns, filters, …); not rendered directly |

```xml
<rect x="20" y="20" width="120" height="80" rx="16" fill="#2563eb"/>
<polygon points="200,20 260,120 140,120" fill="#facc15"/>
<path d="M 20 180 C 60 120 140 120 180 180 Z" fill="none"
      stroke="#14b8a6" stroke-width="8" stroke-linecap="round"/>
```

### Stroke styling

On any 2D shape with a `stroke`:

| Attribute | Values | Default |
|---|---|---|
| `stroke-width` | length | `1` |
| `stroke-linecap` | `butt`, `round`, `square` | `butt` |
| `stroke-linejoin` | `miter`, `round`, `bevel` | `miter` |
| `stroke-miterlimit` | number | `4` |
| `stroke-dasharray` | list of lengths (e.g. `8 4`), or `none` | `none` |
| `stroke-dashoffset` | length | `0` |
| `pathLength` | number — calibrates the dash pattern to a notional path length | actual length |

### Gradients

Define under `<defs>` and reference with `fill="url(#id)"` (or `stroke`).

```xml
<defs>
  <linearGradient id="sky" x1="0" y1="0" x2="0" y2="300" gradientUnits="userSpaceOnUse">
    <stop offset="0"  stop-color="#1b2a4a"/>
    <stop offset="1"  stop-color="#0a1020"/>
  </linearGradient>
  <radialGradient id="glow" cx="50%" cy="50%" r="50%">
    <stop offset="0" stop-color="#ffd9a0"/>
    <stop offset="1" stop-color="#ff7a59" stop-opacity="0"/>
  </radialGradient>
</defs>
<rect width="400" height="300" fill="url(#sky)"/>
<circle cx="200" cy="150" r="120" fill="url(#glow)"/>
```

| Element | Attributes consumed |
|---|---|
| `<linearGradient>` | `x1`, `y1`, `x2`, `y2`, `gradientUnits` |
| `<radialGradient>` | `cx`, `cy`, `r`, `gradientUnits` |
| `<stop>` | `offset` (`0`–`1` or `0%`–`100%`), `stop-color`, `stop-opacity` |

- `gradientUnits` is `objectBoundingBox` (default — coordinates `0`–`1` /
  percentages relative to the shape's bounding box) or `userSpaceOnUse`
  (coordinates in user units).
- Up to 8 stops; offsets are clamped to `[0,1]` and forced non-decreasing.
- **Not supported:** radial focal point (`fx`/`fy`/`fr`), `gradientTransform`,
  `spreadMethod` (always `pad` — the end stops extend), and `href`/`xlink:href`
  inheritance between gradients.

### Patterns

A `<pattern>` tiles a rectangle of solid-color `<rect>` children across the
filled shape.

| Element | Attributes consumed | Notes |
|---|---|---|
| `<pattern>` | `x`, `y`, `width`, `height`, `patternUnits` | Only `<rect>` children (with `fill`) are drawn — up to 8. No nested shapes, gradients, or `patternTransform`/`href`. |

### Markers

Draw a referenced `<marker>` at the vertices of a `<path>`, `<polyline>`,
`<polygon>`, or `<line>`.

```xml
<defs>
  <marker id="arrow" markerUnits="userSpaceOnUse" markerWidth="12" markerHeight="10"
          refX="10" refY="5" orient="auto">
    <path d="M0 0 L12 5 L0 10 Z" fill="#c14b2b"/>
  </marker>
</defs>
<polyline points="14,72 38,28 64,72 86,28" fill="none" stroke="#2563eb"
          stroke-width="5" marker-end="url(#arrow)"/>
```

| Where | Attributes |
|---|---|
| `<marker>` | `markerWidth`, `markerHeight` (default `3`), `refX`, `refY`, `markerUnits` (`strokeWidth` default, or `userSpaceOnUse`), `orient` (`auto`, `auto-start-reverse`, or an angle), `viewBox` |
| on a shape | `marker-start`, `marker-mid`, `marker-end` (each `url(#id)` or `none`); `marker` sets all three |

### Filters

Reference a `<filter>` with `filter="url(#id)"` on any shape. The filter region
defaults to a `-10%…110%` margin around the shape's bounding box (overridable
with `x`/`y`/`width`/`height` on the `<filter>`). Each primitive reads the
standard `in`/`in2`/`result` plumbing and an optional subregion.

Supported primitives:

| Category | Primitives |
|---|---|
| Blur & offset | `feGaussianBlur` (`stdDeviation`), `feOffset` (`dx`, `dy`), `feDropShadow` |
| Color | `feColorMatrix` (`type` = `matrix`/`saturate`/`hueRotate`/`luminanceToAlpha`, `values`), `feComponentTransfer` + `feFuncR/G/B/A` (`type`, `tableValues`, `slope`, `intercept`, `amplitude`, `exponent`, `offset`), `feFlood` (`flood-color`, `flood-opacity`) |
| Noise & distortion | `feTurbulence` (`baseFrequency`, `numOctaves`, `seed`, `type`), `feDisplacementMap` (`scale`, `xChannelSelector`, `yChannelSelector`), `feMorphology` (`operator`, `radius`), `feConvolveMatrix` (`order`, `kernelMatrix`, `divisor`, `bias`, `edgeMode`) |
| Lighting | `feDiffuseLighting`, `feSpecularLighting` with `feDistantLight` (`azimuth`, `elevation`), `fePointLight` (`x`, `y`, `z`), `feSpotLight` |
| Compositing | `feBlend` (`mode`), `feComposite` (`operator`, `k1`–`k4`), `feMerge` + `feMergeNode`, `feTile` |
| Image | `feImage` (`href`/`xlink:href`, including PNG data URLs) |

```xml
<defs>
  <filter id="soft"><feGaussianBlur stdDeviation="3"/></filter>
</defs>
<path d="M 22 74 C 22 22 78 22 78 74 Z" fill="#2563eb" filter="url(#soft)"/>
```

### Clipping & masking

| Element | Reference attribute | Notes |
|---|---|---|
| `<clipPath>` | `clip-path="url(#id)"` | Child shapes define the visible region (hard-edged). Coordinates are user space. |
| `<mask>` | `mask="url(#id)"` | `mask-type` = `luminance` (default — brightness × alpha) or `alpha`. |

```xml
<defs>
  <clipPath id="wedge"><path d="M 18 18 H 82 L 50 82 Z"/></clipPath>
</defs>
<rect x="12" y="12" width="76" height="76" fill="#f2c14e" clip-path="url(#wedge)"/>
```

### Nested `<svg>` viewports

A non-root `<svg>` establishes a child viewport. It honors `x`, `y`, `width`,
`height`, `viewBox` (`min-x min-y width height`), `preserveAspectRatio` (the
`xMidYMid meet` family), and `overflow` (`visible` to disable clipping). This is
also the mechanism behind [3D textures](#an-svg-as-a-3d-texture).

> Note: `viewBox` / `preserveAspectRatio` are honored on **nested** `<svg>`
> elements, **not** on the root — the root maps 1 user unit to 1 output pixel.

### 3D primitives

Require `extension="pupiltong"` on the root. Painted with `fill` (a color or an
[`<svg>` texture](#an-svg-as-a-3d-texture)) and `opacity`; `stroke` and the
stroke properties do not apply. All accept `transform` (see
[3D transforms](#3d-transform-functions)). A negative dimension is an error
(nothing renders); a zero dimension disables just that element.

**`<cube>`** — axis-aligned box.

| Attribute | Meaning | Default |
|---|---|---|
| `cx`, `cy`, `cz` | center | `0` |
| `size` | edge length on all three axes | — |
| `width`, `height`, `depth` | per-axis edge lengths (override `size`) | `size`, else `0` |
| `cube-map` | how an `<svg>` texture wraps the faces: `same` or `cross` | `same` |

**`<ellipsoid>`** — a sphere when the radii are equal.

| Attribute | Meaning | Default |
|---|---|---|
| `cx`, `cy`, `cz` | center | `0` |
| `r` | radius on all three axes | — |
| `rx`, `ry`, `rz` | per-axis radii (override `r`) | `r`, else `0` |

**`<cylinder>`** — right elliptical cylinder, axis along Z.

| Attribute | Meaning | Default |
|---|---|---|
| `cx`, `cy`, `cz` | center | `0` |
| `r` | cap radius (sets `rx`, `ry`, and the default `depth`) | — |
| `rx`, `ry` | per-axis cap radii (override `r`) | `r`, else `0` |
| `depth` | extent along Z | `2r`, else `0` |

**`<surface>`** — a 3D surface built from child `<path>` *cross-section curves*
linked by Bezier patches. The `<surface>`'s own `d` attribute is a chain of
**integer indices** into its `<path>` children (0-based), using a path-like
mini-language:

| Command | Meaning |
|---|---|
| `M i` | start the chain at path `i` |
| `L j` | ruled (flat) patch to path `j` |
| `Q i j` | quadratic patch (control row `i`, end `j`) |
| `C i j k` | cubic patch (control rows `i`, `j`, end `k`) |
| `Z` | close back to the start path |
| `P t r b l` | a Coons patch bounded by four cubic-Bezier paths (top/right/bottom/left) |

All referenced paths must flatten to the same number of samples. The simplest
surface rules one flat strip between two parallel lines:

```xml
<surface d="M 0 L 1" fill="#f2c14e">
  <path d="M 20 30 L 80 30"/>
  <path d="M 20 30 L 80 30" transform="translate(0, 40)"/>
</surface>
```

`<surface>` is the advanced primitive — see [SPEC.md §5.4](SPEC.md) for the full
grammar, and `app-macos`'s built-in scene (a Coons-patch sphere) for a worked
example.

### 3D transform functions

In the `transform` attribute (only when the extension is enabled; mixable with
the SVG 1.1 functions `translate`, `scale`, `rotate`, `skewX`, `skewY`,
`matrix`). Angles take `deg` (default), `rad`, `grad`, or `turn`.

| Function | Effect |
|---|---|
| `rotateX(a)`, `rotateY(a)`, `rotateZ(a)` | rotate about an axis (`rotateZ` ≡ SVG `rotate`) |
| `rotate3d(x, y, z, a)` | rotate by `a` about the axis `(x, y, z)` |
| `translate3d(tx, ty, tz)`, `translateZ(tz)` | translate (incl. depth) |
| `scale3d(sx, sy, sz)`, `scaleZ(sz)` | scale (incl. depth) |
| `matrix3d(m11 … m44)` | a full 4×4 matrix, **column-major** |

### An `<svg>` as a 3D texture

Give a non-root `<svg>` an `id` and reference it from a 3D primitive's `fill`
with `url(#id)`. Its content is rasterised once (into its `viewBox`, or its
`width`/`height`) and sampled over the surface with a per-primitive UV map:

- **`<cube>`** — `cube-map="same"` shows the whole texture upright on every
  face; `cube-map="cross"` reads the faces from a 4×3 horizontal-cross atlas
  (`+Y` top; `-X / +Z / +X / -Z` middle row; `-Y` bottom).
- **`<ellipsoid>`** — equirectangular wrap (like a world map onto a globe).
- **`<cylinder>`** — the side wall wraps once around; the caps use a polar-disk
  projection.
- **`<surface>`** — each Bezier patch maps the texture across its `(u, v)`.

Transparent texels stay transparent (no implicit background).

```xml
<svg xmlns="http://www.w3.org/2000/svg" extension="pupiltong" width="512" height="512">
  <defs>
    <!-- A 4×3 horizontal-cross atlas: one colored square per cube face. -->
    <svg id="faces" viewBox="0 0 400 300">
      <rect x="100" y="0"   width="100" height="100" fill="#ffd9a0"/>
      <rect x="0"   y="100" width="100" height="100" fill="#f4a259"/>
      <rect x="100" y="100" width="100" height="100" fill="#ff9f6b"/>
      <rect x="200" y="100" width="100" height="100" fill="#e76f51"/>
      <rect x="300" y="100" width="100" height="100" fill="#c1495f"/>
      <rect x="100" y="200" width="100" height="100" fill="#8a4f7d"/>
    </svg>
  </defs>
  <cube cx="0" cy="0" cz="0" size="260" cube-map="cross"
        transform="translate(256 256) rotateY(35deg) rotateX(-22deg)"
        fill="url(#faces)"/>
</svg>
```

## Limitations & gotchas

A consolidated list of what to expect — most are mentioned in context above.

- **Colors are hex + ~20 named colors.** No `rgb()`, `hsl()`, `#rrggbbaa`, or
  most CSS color names. Unknown `fill` → black.
- **Styling is via presentation attributes**, not the CSS `style` attribute
  (which is read only for `<stop>` and nested-`<svg>` `overflow`). No CSS
  selectors / classes / cascade yet.
- **Root `viewBox` is ignored** — 1 user unit = 1 output pixel. Author for the
  exact `--width × --height`. (`viewBox` works on *nested* `<svg>`.)
- **Lengths** are unitless / `px` / `%` only.
- **Gradients** lack focal points, `gradientTransform`, `spreadMethod`, and
  `href` inheritance; **patterns** tile only solid `<rect>` children.
- **3D primitives** take `fill` + `opacity` only — no `stroke`, and `clip-path`
  / `mask` / `filter` are not defined on them.
- **No `<text>`, `<image>`, `<use>`, `<tspan>`, or `<symbol>`** — these parse but
  render as empty. (Raster images are only reachable via `feImage` data URLs.)
- **Full-frame 2D backdrops + 3D** don't always depth-compose cleanly; prefer a
  transparent background for 3D scenes.
- **Rendering needs a GPU.** Use [`--check`](#cli-reference) to validate parsing
  without one.

## CLI reference

```sh
svg3 <INPUT> [-o OUT.png] [-W WIDTH] [-H HEIGHT] [--camera] [--check]
```

| Argument | Meaning | Default |
|---|---|---|
| `<INPUT>` | document file, or `-` to read from stdin | — |
| `-o, --output <FILE>` | output PNG (defaults to `<input>.png`; **required** for stdin) | derived |
| `-W, --width <PX>` | output width in pixels | `512` |
| `-H, --height <PX>` | output height in pixels | `512` |
| `--camera` | view 3D content through a perspective camera (no effect on plain 2D) | off |
| `--check` | parse & report without rendering — **needs no GPU** | off |

```sh
svg3 logo.svg                              # → logo.png (512×512, orthographic)
svg3 scene.svg3 -o out.png -W 800 -H 800   # custom size
svg3 cube.svg3 -o cube.png --camera        # perspective 3D
printf '%s' "$SVG" | svg3 - -o out.png     # read from stdin
svg3 doc.svg3 --check                      # validate parsing only
```

## Generate images from your AI coding agent

The [`svg3-skill/`](svg3-skill/) directory is a portable **agent skill**: drop
it into your coding agent (e.g. Claude Code's skills directory) and it teaches
the agent the svg3 3D extension, how to author a document, render it with the
`svg3` CLI, view the PNG, and iterate — so "make me an image of …" becomes a
tight author → render → look loop. See [`svg3-skill/SKILL.md`](svg3-skill/SKILL.md).

## Interactive macOS app

Prefer something interactive? `cargo run -p app-macos` opens a native macOS
window (`svg3-macos`) where you paste SVG/svg3 text and watch it render live.
For 3D scenes it gives you an orbiting perspective camera: **drag or arrow keys
to orbit, scroll to zoom, `R` to reset**.

## How it works

```
svg3 XML   ──►  svg3::dom    ──►  svg3::render ──►  PNG / GPU surface
(text)          (element tree)    (wgpu draw)
```

A Stylo-driven `svg3::style` cascade is a paused roadmap item; until it lands,
presentation attributes are read directly in `svg3::render`.

## Workspace

| Crate       | Responsibility                                                       |
|-------------|----------------------------------------------------------------------|
| `svg3`      | The library: `dom` (parse svg3/SVG XML into an element tree) and `render` (turn a parsed document into GPU draw calls and a headless image). |
| `svg3-cli`  | The headless `svg3` command — render a document to a PNG. |
| `app-macos` | Native macOS demo (`svg3-macos`): paste SVG text and render it in a wgpu window. |

## Build & run

The Rust toolchain is pinned in [`rust-toolchain.toml`](rust-toolchain.toml)
(`nightly-2026-04-20`); `rustup` selects it automatically inside this repo.

```sh
cargo build --workspace          # first build is slow (wgpu)
cargo test  --workspace
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings

cargo run -p svg3-cli -- input.svg -o out.png   # headless render to PNG
cargo run -p app-macos                          # interactive macOS demo
bash scripts/bundle-macos.sh                     # build target/release/bundle/svg3-macos.app

cargo bench --workspace                          # criterion benches (codspeed-instrumented)
```

## Roadmap

1. **(done)** winit event loop + wgpu surface — the macOS demo renders pasted SVG.
2. **(done)** svg3/SVG XML parsing → element tree.
3. **(done)** 2D shapes, strokes, gradients, patterns, markers, filters,
   `clipPath`/`mask`, nested viewports, and headless image output.
4. **(done)** 3D primitive mesh generation and depth-aware 2D/3D compositing,
   including nested-SVG texture paint servers.
5. **(paused)** Stylo CSS cascade — computed styles drive material/transform.
6. Multi-platform native demos (Windows, Linux) + Linux/Windows CI.
7. Native macOS `.app` bundling.

## Contributing

See [AGENTS.md](AGENTS.md) for toolchain, build/test/lint commands, and
conventions (this repo collaborates with multiple AI agents through that single
canonical guide).

## License

[MPL-2.0](LICENSE).
