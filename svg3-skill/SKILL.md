---
name: svg3-render
description: >-
  Render an svg3 or SVG document to a PNG image with the `svg3` command-line
  renderer, then view the result to verify it. Use this whenever the user wants
  to generate an image, render an SVG, produce or preview a PNG, see what an
  svg3/SVG document looks like, visualize a <cube>/<ellipsoid>/<cylinder>/
  <surface> or other svg3 3D scene, or iterate on a vector document until it
  looks right.
---

# svg3-render — generate images with the `svg3` CLI

`svg3` is a headless renderer: it turns an svg3/SVG document (text) into a PNG.
svg3 is SVG 1.1 **plus an opt-in 3D extension**, so the same tool renders
ordinary 2D SVG *and* three-dimensional `<cube>` / `<ellipsoid>` / `<cylinder>`
/ `<surface>` scenes on the GPU.

**The 3D extension is specific to svg3 — it is not standard SVG, so do not rely
on outside knowledge for it. This skill teaches it below; follow it exactly.**

The loop is always: **author a document → render it with `svg3` → view the PNG
→ iterate.**

## 1. Make sure the `svg3` CLI is installed

The renderer is the `svg3-cli` crate; it installs a binary named `svg3`.

```sh
svg3 --version                 # already installed?
cargo install --path svg3-cli  # …or build it from a checkout of the svg3 repo
```

Run `svg3 --help` to see every flag.

## 2. Author the document

The root element is always `<svg>` with a pixel `width`/`height`. Build content
inside it. **Pixels map 1:1 to user units** (the root `viewBox` is not applied),
so author for the exact size you will render — a 512×512 render shows user space
`0..512 × 0..512`, origin top-left, +X right, +Y down.

### 2a. Plain 2D SVG

Standard SVG 1.1: `<rect>`, `<circle>`, `<ellipse>`, `<line>`, `<polyline>`,
`<polygon>`, `<path>`, grouped with `<g>`, plus gradients, patterns, markers,
filters, `<clipPath>`/`<mask>`, and nested `<svg>` viewports. Shapes paint in
document order (later on top). See [`examples/shapes.svg`](examples/shapes.svg).

```xml
<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512">
  <rect width="512" height="512" fill="#0a1020"/>
  <circle cx="256" cy="256" r="150" fill="#ff7a59" opacity="0.9"/>
  <path d="M 96 380 C 180 280 332 280 416 380" fill="none"
        stroke="#7ad7f0" stroke-width="10" stroke-linecap="round"/>
</svg>
```

### 2b. The svg3 3D extension

To use 3D, add **`extension="pupiltong"`** to the root `<svg>`. **Without that
attribute the 3D elements draw nothing** — this is the single most common
mistake. Save 3D documents as `.svg3` (plain SVG stays `.svg`). See
[`examples/cube.svg3`](examples/cube.svg3).

**The coordinate system gains a Z axis: +X right, +Y down, +Z toward the
viewer** (left-handed). 2D content stays in the plane `z = 0`. Greater Z is
closer to the camera and occludes lesser Z.

**The four primitives** are centered at `(cx, cy, cz)` and painted with `fill`
and `opacity` (no `stroke`). Copy-pasteable, render-ready bodies — drop any of
these inside a `<svg extension="pupiltong" width="512" height="512">` root:

```xml
<!-- CUBE. `size` = all edges; or set width/height/depth independently. -->
<cube cx="256" cy="256" cz="0" size="220" fill="#f2c14e"/>

<!-- ELLIPSOID (sphere when rx=ry=rz). `r` sets all three radii. -->
<ellipsoid cx="256" cy="256" cz="0" r="140" fill="#10b981"/>

<!-- CYLINDER, axis along Z. `depth` = length, `r` = cap radius. -->
<cylinder cx="256" cy="256" cz="0" r="120" depth="180" fill="#c14b2b"/>

<!-- SURFACE: child <path> cross-sections linked by Bezier patches.
     "M 0 L 1" rules a flat strip between path 0 and path 1. -->
<surface d="M 0 L 1" fill="#ec4899">
  <path d="M 160 200 L 352 200"/>
  <path d="M 160 200 L 352 200" transform="translate(0, 120)"/>
</surface>
```

3D primitive attributes at a glance:

| Element | Attributes |
|---|---|
| `<cube>` | `cx cy cz`, `size` **or** `width`/`height`/`depth`, `cube-map` (`same`\|`cross`) |
| `<ellipsoid>` | `cx cy cz`, `r` **or** `rx`/`ry`/`rz` |
| `<cylinder>` | `cx cy cz`, `r` **or** `rx`/`ry`, `depth` |
| `<surface>` | `d` (index chain: `M i`, `L j`, `Q i j`, `C i j k`, `Z`, `P t r b l`) + child `<path>` curves |

### 2c. Transforms — and the rotate-about-origin gotcha

Position and orient with the `transform` attribute. svg3 adds 3D functions to
the SVG 1.1 set, mixable in one attribute and composed left-to-right:

- 3D: `rotateX(a)`, `rotateY(a)`, `rotateZ(a)`, `rotate3d(x,y,z,a)`,
  `translate3d(tx,ty,tz)`, `translateZ(tz)`, `scale3d(...)`, `scaleZ(s)`,
  `matrix3d(...)`. Angles take `deg` (default), `rad`, `grad`, `turn`.
- 2D (still available): `translate`, `scale`, `rotate`, `skewX`, `skewY`, `matrix`.

**Gotcha: transforms rotate about the origin `(0,0,0)`, not the shape's center.**
To spin a primitive in place, build it at the origin (`cx=cy=cz=0`) and *then*
translate it into the frame:

```xml
<!-- Build at origin → rotate → move to the center of a 512² frame: -->
<cube cx="0" cy="0" cz="0" size="220"
      transform="translate(256 256) rotateY(35deg) rotateX(-22deg)"
      fill="#e8743b"/>
```

### 2d. Texturing a primitive with a nested `<svg>`

A primitive's `fill` can reference a nested `<svg>` (by `id`) as a paint server;
its rasterized content wraps over the surface as a texture. For a cube, set
`cube-map="cross"` and lay the texture out as a 4×3 horizontal cross (`+Y` top;
`-X / +Z / +X / -Z` middle row; `-Y` bottom), or `cube-map="same"` to show the
whole texture on every face. Ellipsoid wraps equirectangularly; cylinder wraps
the side wall + polar-disk caps. See [`examples/cube.svg3`](examples/cube.svg3).

### 2e. Shade a 3D primitive with a lighting filter

**The engine does no lighting on 3D primitives**, so a solid-`fill` `<ellipsoid>`
is just a flat disc. To fake rounded shading, apply an SVG **lighting filter**:
blur the silhouette's `SourceAlpha` into a height field, light it with
`feDiffuseLighting` (+ optional `feSpecularLighting`), and composite over the
fill. It shades the primitive's *projected silhouette*, so the same filter works
on every primitive (and on 2D shapes). Drop this `gloss` filter in `<defs>` and
reference it with `filter="url(#gloss)"`:

```xml
<filter id="gloss">
  <feGaussianBlur in="SourceAlpha" stdDeviation="8" result="bump"/>
  <feDiffuseLighting in="bump" surfaceScale="10" diffuseConstant="0.95"
                     lighting-color="#fff4dc" result="diff">
    <feDistantLight azimuth="135" elevation="58"/>
  </feDiffuseLighting>
  <feComposite in="diff" in2="SourceGraphic" operator="arithmetic"
               k1="1" k2="0" k3="0" k4="0" result="shaded"/>
  <feSpecularLighting in="bump" surfaceScale="10" specularConstant="0.6"
                      specularExponent="26" lighting-color="#ffffff" result="spec">
    <feDistantLight azimuth="135" elevation="58"/>
  </feSpecularLighting>
  <feComposite in="spec" in2="SourceGraphic" operator="in" result="spec-in"/>
  <feComposite in="spec-in" in2="shaded" operator="arithmetic"
               k1="0" k2="1" k3="1" k4="0"/>
</filter>
...
<ellipsoid cx="256" cy="256" cz="0" r="140" fill="#5fa98a" filter="url(#gloss)"/>
```

Pair the filter with a **solid `fill`** (not a gradient — see the gotchas).
Because each filtered element composites as its own layer in document order,
build an overlapping 3D scene **back-to-front** (e.g. a teapot's handle and
spout *before* the body; the lid and knob *after*).

### 2f. Tubes & lathed shapes from one `<surface>`

A `<surface>` lofts its child `<path>` cross-sections, and **each child flattens
in its own local coordinates _before_ its `transform` applies.** So the reliable
way to build a smooth tube, spout, handle, or vase is to **reuse one canonical
cross-section `d`** (e.g. a circle) for every ring and vary only each ring's
`transform` — `scale` for radius, rotations to orient, `translate3d` to place.
Every ring then flattens to the *same* sample count, satisfying surface's "all
cross-sections must share a sample count" rule for free (varying the radius
inside the `d` instead would change the count and silently void the surface).

To stand a circle perpendicular to a centreline whose tangent points at angle β
in the XY plane, orient it `rotateZ(β+90) rotateX(90)`:

```xml
<!-- a tapering tube: 3 rings of one unit circle, lofted M 0 L 1 L 2 -->
<surface d="M 0 L 1 L 2" fill="#5fa98a" filter="url(#gloss)">
  <path d="M 40 0 C 40 22.1 22.1 40 0 40 C -22.1 40 -40 22.1 -40 0 C -40 -22.1 -22.1 -40 0 -40 C 22.1 -40 40 -22.1 40 0 Z"
        transform="translate3d(180,360,0) rotateZ(70) rotateX(90) scale(1.0)"/>
  <path d="M 40 0 C 40 22.1 22.1 40 0 40 C -22.1 40 -40 22.1 -40 0 C -40 -22.1 -22.1 -40 0 -40 C 22.1 -40 40 -22.1 40 0 Z"
        transform="translate3d(250,300,0) rotateZ(55) rotateX(90) scale(0.7)"/>
  <path d="M 40 0 C 40 22.1 22.1 40 0 40 C -22.1 40 -40 22.1 -40 0 C -40 -22.1 -22.1 -40 0 -40 C 22.1 -40 40 -22.1 40 0 Z"
        transform="translate3d(300,250,0) rotateZ(40) rotateX(90) scale(0.4)"/>
</surface>
```

See [`examples/teapot.svg3`](examples/teapot.svg3) — its spout and handle are
tubes built exactly this way, on a filter-shaded `<ellipsoid>` body.

## 3. Render

```sh
svg3 logo.svg                          # → logo.png  (512×512, orthographic)
svg3 scene.svg -o out.png -W 800 -H 800
svg3 cube.svg3 -o cube.png --camera    # add a perspective camera
printf '%s' "$SVG" | svg3 - -o out.png # read the document from stdin
```

`--camera` views the scene through a **perspective** camera that frames the
`width × height` region, so **center 3D content at `(width/2, height/2)`**.
Without it, 3D still renders — just in **orthographic** (parallel) projection,
with no perspective foreshortening. Plain 2D documents render head-on either way.

## 4. View the result, then iterate

**Always open the produced PNG and look at it** with your image-viewing tool (in
Claude Code, the Read tool renders images). A zero exit code only means a file
was written — it does not tell you the picture is correct. Then edit the
document, re-render, and repeat until it looks right.

## Constraints & gotchas (read before authoring)

These trip people up because svg3 supports a *subset* of SVG and a non-standard
3D layer. Author within them from the start:

- **Opt into 3D:** 3D elements/transforms do nothing without
  `extension="pupiltong"` on the root.
- **Colors are hex or a small named set only.** Use `#rgb` / `#rrggbb`, or:
  `black white red green blue yellow cyan/aqua magenta/fuchsia gray/grey silver
  maroon navy orange purple lime teal`. **`rgb()`, `hsl()`, `#rrggbbaa`, and
  other CSS color names (e.g. `skyblue`, `crimson`) are NOT supported** — an
  unknown `fill` falls back to black. Convert any other color to hex.
- **Style via attributes, not CSS.** Write `fill="#ff0000"`, never
  `style="fill:#ff0000"`. The `style` attribute is ignored except on `<stop>`
  and nested-`<svg>` `overflow`.
- **For transparency**, use `opacity` / `fill-opacity` / `stroke-opacity` (or
  `stop-opacity` on gradient stops), not an alpha hex.
- **Pixels are 1:1** and the root `viewBox` is ignored — author for the exact
  `-W`/`-H`. (`viewBox` works on *nested* `<svg>`.)
- **3D primitives are flat-filled — there is no scene lighting.** A bare
  `<ellipsoid fill="#5fa98a"/>` renders as a flat disc, not a sphere. To make 3D
  shapes look round you **must** shade them — apply a lighting `filter` (§2e).
  `stroke`, `clip-path`, and `mask` still do nothing on 3D primitives.
- **Don't use a gradient/texture `fill` to *shade* a 3D primitive.** The paint
  maps through the primitive's UV, so a gradient reveals the seam of an
  ellipsoid's equirectangular wrap and *tiles per-patch* on a `<surface>`
  (stripes along a tube). For rounded form use a **solid `fill` + a lighting
  filter** (§2e); reserve textures (§2d) for surface *imagery*.
- **Background is transparent.** In 2D, add a backdrop `<rect>` if you want one.
  In a 3D scene prefer leaving it transparent — a full-frame 2D backdrop and 3D
  primitives don't depth-compose cleanly.
- **Not supported:** `<text>`, `<image>`, `<use>`, `<tspan>`, `<symbol>` (they
  render empty). No raster images except via `feImage` PNG data URLs. Gradients
  have no focal point / `gradientTransform` / `spreadMethod`; patterns tile only
  solid `<rect>` children.

For the full formal language read **SPEC.md**; for the complete supported-feature
reference read the project **README.md** — both at
<https://github.com/PupilTong/svg3>.

## Validate without a GPU

Rendering needs a GPU adapter. On a headless host with none, the render
commands fail with a clear message. To check a document parses — on any host,
no GPU and no PNG — use `--check`:

```sh
svg3 doc.svg --check     # prints "<doc>: parsed OK; would render at WxH"
```

## Worked examples (bundled with this skill)

- [`examples/cube.svg3`](examples/cube.svg3) — a textured 3D cube. Its `fill`
  references a nested `<svg>` *paint server* and `cube-map="cross"` wraps that
  texture across the six faces. `svg3 examples/cube.svg3 -o cube.png --camera`.
- [`examples/teapot.svg3`](examples/teapot.svg3) — a ceramic teapot: `<ellipsoid>`
  body/lid/knob plus `<surface>` tube spout & handle (§2f), all shaded by one
  `gloss` lighting filter (§2e) over solid fills, on a pushed-back backdrop.
  `svg3 examples/teapot.svg3 -o teapot.png`.
- [`examples/shapes.svg`](examples/shapes.svg) — plain 2D: overlapping gradient
  circles with opacity. `svg3 examples/shapes.svg -o shapes.png`.

## Flags

| Argument | Meaning | Default |
|---|---|---|
| `<INPUT>` | document file, or `-` to read stdin | — |
| `-o, --output <FILE>` | output PNG (defaults to `<input>.png`; required for stdin) | derived |
| `-W, --width <PX>` | output width in pixels | 512 |
| `-H, --height <PX>` | output height in pixels | 512 |
| `--camera` | add a perspective camera (only affects `pupiltong` 3D) | off |
| `--check` | parse and report without rendering (no GPU) | off |
