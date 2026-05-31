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
svg3 is SVG 1.1 plus an opt-in 3D extension, so the same tool renders ordinary
2D SVG *and* three-dimensional `<cube>` / `<ellipsoid>` / `<cylinder>` /
`<surface>` scenes on the GPU.

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

- The root element is always `<svg>`.
- **2D:** plain SVG 1.1 — `<rect>`, `<circle>`, `<path>`, gradients, opacity,
  filters, clip/mask, nested `<svg>`. See [`examples/shapes.svg`](examples/shapes.svg).
- **3D:** add `extension="pupiltong"` to the root `<svg>`, then use the 3D
  elements and 3D transforms (`rotateX/Y/Z`, `translate3d`, `scale3d`, …).
  **Without that attribute the 3D elements draw nothing.** See
  [`examples/cube.svg`](examples/cube.svg).

Three things that commonly trip people up — keep them in mind while authoring:

- **Pixels map 1:1 to user units.** The output is `--width × --height` pixels
  and the document's coordinate grid lands straight on them (no `viewBox`
  scaling yet). Author content for the size you'll render — e.g. a 512×512
  render shows user space `0..512 × 0..512`.
- **Transforms rotate about the origin** (just like SVG 1.1). To spin a shape
  about its own centre, place it at the origin and translate it into position:
  `transform="translate(256 256) rotateY(35deg) rotateX(-22deg)"`.
- **The background is transparent.** In 2D, add a backdrop `<rect>` if you want
  one. In a 3D (`pupiltong`) scene, prefer leaving it transparent — a
  full-frame 2D backdrop and 3D primitives don't depth-compose cleanly yet.

For the full language read **SPEC.md**; for the currently-supported subset read
the project **README.md** — both at <https://github.com/PupilTong/svg3>.

## 3. Render

```sh
svg3 logo.svg                          # → logo.png  (512×512, orthographic)
svg3 scene.svg -o out.png -W 800 -H 800
svg3 cube.svg  -o cube.png --camera    # add a perspective camera
printf '%s' "$SVG" | svg3 - -o out.png # read the document from stdin
```

`--camera` views the scene through a perspective camera that frames the
`width × height` region, so centre 3D content at `(width/2, height/2)`. Without
it, 3D still renders — just in **orthographic** (parallel) projection, with no
perspective foreshortening.

## 4. View the result, then iterate

**Always open the produced PNG and look at it** with your image-viewing tool (in
Claude Code, the Read tool renders images). A zero exit code only means a file
was written — it does not tell you the picture is correct. Then edit the
document, re-render, and repeat until it looks right.

## Validate without a GPU

Rendering needs a GPU adapter. On a headless host with none, the render
commands fail with a clear message. To check a document parses — on any host,
no GPU and no PNG — use `--check`:

```sh
svg3 doc.svg --check     # prints "<doc>: parsed OK; would render at WxH"
```

## Worked examples (bundled with this skill)

- [`examples/cube.svg`](examples/cube.svg) — a textured 3D cube. Its `fill`
  references a nested `<svg>` *paint server* and `cube-map="cross"` wraps that
  texture across the six faces. `svg3 examples/cube.svg -o cube.png --camera`.
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
