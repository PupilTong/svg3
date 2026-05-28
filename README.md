# svg3

**An extended SVG renderer for 3D models.**

`svg3` is an opt-in extension to SVG 1.1: it inherits SVG 1.1's `<svg>` root
and 2D drawing model, and documents enable the 3D extension with
`extension="pupiltong"` on the root `<svg>`. Without that attribute, the
renderer behaves as a normal SVG renderer and ignores svg3-only elements and
3D transform functions. With it, documents can use three-dimensional graphics
elements (`<cube>`, `<ellipsoid>`, `<cylinder>`, …) plus 3D transform
functions, rendered on the GPU. Styling stays faithful to the web platform:
element styles are resolved with
[Stylo](https://crates.io/crates/stylo) (Servo's CSS engine), and the
resulting scene is painted with [wgpu](https://crates.io/crates/wgpu).

The project is built from the ground up on the current Rust graphics ecosystem.
Demos will target multiple platforms; a native macOS desktop app is the first.

The document language is specified in [SPEC.md](SPEC.md) (editor's draft).

## Pipeline

```
svg3 XML   ──►  svg3::dom   ──►  svg3::style  ──►  svg3::render ──►  GPU surface
(text)          (element tree)   (Stylo cascade)   (wgpu draw)
```

## Workspace

| Crate       | Responsibility                                                       |
|-------------|----------------------------------------------------------------------|
| `svg3`      | The library. Three submodules: `dom` (parse SVG/svg3 XML into a mutable element tree, SVG 1.1 root + opt-in 3D extensions), `style` (resolve computed styles via Stylo), and `render` (turn a parsed document into GPU draw calls with wgpu). |
| `app-macos` | Native macOS demo: paste SVG text, then render supported shapes in a wgpu window. |

> **Status: early scaffolding.** `app-macos` opens a real Metal-backed Cocoa
> window, prompts for an SVG string, and renders the currently supported
> filled/stroked `<rect>` / `<circle>` / `<ellipse>` / `<polygon>` /
> `<polyline>` / stroked `<line>` / filled/stroked `<path>` 2D geometry
> plus the svg3 3D `<cube>`, `<ellipsoid>`, `<cylinder>`, and `<surface>`
> primitives when the root has `extension="pupiltong"`, plus nested plain-SVG
> `<svg>` viewports in normal 2D mode. Click the window, or press Command+O / Command+I, to edit
> the SVG input again.
> `svg3::dom` parses svg3 XML into an element tree with raw attributes
> (roadmap item 2) — currently outer and nested `<svg>`, `<g>`, `<defs>`, `<rect>`,
> `<circle>`, `<ellipse>`, `<polygon>`, `<polyline>`, `<line>`, `<path>`,
> `<filter>`, every supported `<fe…>` primitive (`feGaussianBlur`, `feImage`,
> `feColorMatrix`, `feTurbulence`, `feSpecularLighting`, `feDiffuseLighting`,
> `feMorphology`, `feFlood`, `feDropShadow`, `feDisplacementMap`,
> `feConvolveMatrix`, `feComponentTransfer`) and their child elements
> (`feFuncR/G/B/A`, `feDistantLight`, `fePointLight`, `feSpotLight`),
> `<marker>`, `<clipPath>`, `<mask>`, `<linearGradient>`,
> `<radialGradient>`, `<stop>`, `<pattern>`, `<cube>`, `<ellipsoid>`,
> `<cylinder>`, and `<surface>` are recognised; the rest of SVG 1.1
> round-trips as unknown elements. `svg3::render` tessellates fills and
> strokes for `<rect>`, `<circle>`, `<ellipse>`, `<polygon>`, `<polyline>`,
> and `<path>`, plus stroke-only `<line>` geometry, including stroke caps,
> joins, dashes, `pathLength` dash calibration, opacity attributes, and
> referenced SVG markers and paint servers (`<linearGradient>`,
> `<radialGradient>`, and rectangular-child `<pattern>` tiles), plus svg3's
> 3D primitives only for documents whose root has `extension="pupiltong"`,
> then rasterises them headlessly to an image
> (roadmap items 3 and 4), using root `<svg width>` / `<svg height>` as the
> initial percentage viewport. In normal 2D SVG mode, nested `<svg>` elements
> establish child viewports with `x`/`y`, `width`/`height`, `viewBox`,
> `preserveAspectRatio`, and default overflow clipping. Filters and
> `clip-path` inside a clipped nested viewport are currently flattened as raw
> geometry until nested render-plan composition lands. The svg3 3D `<ellipsoid>` primitive (SPEC §5.3)
> tessellates the implicit surface to a UV-parameterised mesh, and
> `<cylinder>` (SPEC §5.5) tessellates cap and side-wall triangles alongside
> `<cube>`.
> Headless rendering also applies referenced `<filter>` elements composed
> of an ordered chain of `<fe…>` primitives, with each primitive
> implemented as a dedicated GPU fragment-shader pass over offscreen
> ping/pong textures (no CPU filter fallback). PNG data-URL `<feImage>`
> primitives are decoded and uploaded lazily when a referenced filter
> paints. Referenced `<clipPath>` definitions render supported child
> geometry into an alpha mask before filtering, and referenced `<mask>`
> definitions render supported child geometry as luminance masks by default
> or alpha masks with `mask-type="alpha"`. `clipPathUnits`,
> `maskUnits`, and `maskContentUnits` are currently limited to the
> default user-space behavior; `objectBoundingBox` mapping is still to come.
> Nested filter, clip, or mask references inside those definition subtrees
> are flattened as raw geometry until nested render-plan composition lands.
> The Stylo
> cascade is still a skeleton — see the roadmap below.

## Toolchain

The Rust toolchain is pinned in [`rust-toolchain.toml`](rust-toolchain.toml)
(`nightly-2026-04-20`). `rustup` selects it automatically inside this repo.

## Build & run

```sh
cargo build --workspace          # first build is slow (stylo + wgpu)
cargo test  --workspace
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings

cargo run -p app-macos                       # paste SVG text into the native macOS demo
bash scripts/bundle-macos.sh                 # build a .app: target/release/bundle/svg3-macos.app

cargo bench --workspace                      # criterion benches (codspeed-instrumented)
```

## Roadmap

1. **(done)** winit event loop + wgpu surface — the `app-macos` crate opens a native window, accepts SVG text, and renders supported 2D shapes.
2. **(done)** SVG/svg3 XML parsing → element tree (`<svg>` root with `<g>`, `<cube>`, `<ellipsoid>`, `<cylinder>`, and `<surface>`; svg3 3D rendering requires root `extension="pupiltong"` per [SPEC.md](SPEC.md)).
3. **(done)** 2D basic shapes — tessellate fills and strokes for `<rect>`, `<circle>`, `<ellipse>`, `<polygon>`, `<polyline>`, and `<path>`, plus stroke-only `<line>` geometry (with percentage lengths, nested `<svg>` viewports, caps, joins, dashes, `pathLength` dash calibration, opacity attributes, and referenced markers), apply referenced `<filter>` chains composed from any supported filter primitive, apply referenced `<clipPath>` and `<mask>` definitions through GPU mask passes, and render the result headlessly to an image (`svg3::render`).
4. **(done)** 3D primitive mesh generation. `<cube>` is tessellated to six rectangular faces (12 triangles, 8 corners), `<ellipsoid>` to a UV-parameterised mesh (16 latitude bands × 32 longitude segments) of its implicit surface, and `<cylinder>` to segmented caps and a side wall; all render through a unified depth-aware (`LessEqual`) shape pipeline so per SPEC §7.3 spatial Z resolves occlusion between 2D and 3D content. 2D shapes are biased forward by a tiny per-shape Z stride so coplanar 2D content stays painter-ordered without z-fighting; 3D content keeps its authored world Z. SDF shapes discard transparent fragments so their bounding-quad corners don't write spurious depth. Filtered content participates in depth too — the composite shader samples the filter source's per-pixel depth and writes it as `frag_depth`, so a filtered rect at `z = 0` correctly occludes a 3D primitive behind it and a 3D primitive in front of `z = 0` paints over a later filtered rect. Stylo-driven materials remain.
5. Real Stylo integration (computed styles drive material/transform).
6. Multi-platform native demos (Windows, Linux) + Linux/Windows CI.
7. Native macOS `.app` bundling.

## License

[MPL-2.0](LICENSE).
