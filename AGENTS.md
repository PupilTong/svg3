# Agent / Contributor Instructions

This repository supports LLM-based assistants. The working language is English.

`AGENTS.md` is the single canonical agent-instructions file, shared across tools (Claude Code and GPT/Codex both read it). There is no `CLAUDE.md` — edit this file instead.

## General guidelines

- **Rebase onto `origin/main` before starting any work** to avoid merge conflicts and stale base commits.
- Keep changes minimal and focused.
- Follow existing style conventions and formatting.
- Rust **2021** edition; the toolchain is pinned by [`rust-toolchain.toml`](rust-toolchain.toml) (`nightly-2026-04-20`). `rustup` selects it automatically in this repo.
- Use cargo workspace conventions (`*.workspace = true` for shared metadata and dependencies; declare versions once in the root `[workspace.dependencies]`).
- Update or add tests when relevant. Always run tests. For renderer features
  that affect visual output, add enough snapshot cases to cover each supported
  feature point, not just pixel-probe unit tests.
- **Test behavior, not implementation.** Prefer end-to-end paths (parse a document, run it through the pipeline, assert on output) over asserting on internal helpers or intermediate structures.
- Error handling returns specific typed errors (`thiserror`); avoid stringly-typed errors.
- All `unsafe` blocks must have a `// SAFETY:` comment explaining the upheld invariant.
- Prefer `pub(crate)` or private by default; only make items `pub` when they are part of a crate's public API.

## Formatting

- CI runs `cargo fmt --check` using the toolchain pinned in `rust-toolchain.toml`, which is the same toolchain `rustup` selects locally — so local and CI formatting agree as long as you don't override the toolchain.
- Always run `cargo fmt --check` (not just `cargo fmt`) before committing.

## Testing

- `cargo test --workspace` runs the unit and integration tests. The GPU-backed tests in `svg3::render` — the headless render test, `tests/filter.rs`, `tests/snapshot.rs`, and `tests/wpt_filter_cases.rs` — self-skip when no GPU adapter is available, so they run on macOS but not on a GPU-less host.
- **Snapshot tests:** `svg3/tests/snapshot.rs` renders filled/stroked `<rect>`, `<circle>`, `<ellipse>`, `<polygon>`, `<polyline>`, stroked `<line>`, filled/stroked `<path>`, referenced `<marker>` documents, SVG paint servers (`<linearGradient>`, `<radialGradient>`, `<pattern>`, `<stop>`, fill/stroke references, stop opacity/style, object-bounding-box/user-space units, and paint feeding filters), the svg3 3D `<cube>`, `<ellipsoid>`, `<cylinder>`, and `<surface>` (both orthographic projection and perspective camera), and the full referenced-filter set headlessly and compares them against the committed golden PNGs in `svg3/tests/snapshots/` (the reviewable images). The filter coverage spans every supported primitive — `feGaussianBlur` (multiple axis configurations and source shapes), PNG data-URL `feImage` (explicit / intrinsic / percentage sizing, `href` and `xlink:href`, alpha, blur, painter order, `preserveAspectRatio` (default `xMidYMid meet`, `none` for stretching), and skipped unsupported / invalid cases), `feColorMatrix` (saturate / luminanceToAlpha / hueRotate), `feTurbulence` (turbulence and fractalNoise), `feSpecularLighting`, `feDiffuseLighting`, `feMorphology` (dilate / erode), `feFlood`, `feDropShadow`, `feDisplacementMap`, `feConvolveMatrix` (with `edgeMode` duplicate / wrap / none), `feComponentTransfer`, `feOffset`, `feMerge` (multi-node painter-order composite), `feBlend` (normal / multiply / screen / darken / lighten), `feComposite` (over / in / out / atop / xor / arithmetic), `feTile`, plus chained primitive cases. The 3D goldens additionally exercise the 2D-3D depth occlusion path (rect+cube, rect+ellipsoid, rect+cylinder, and rect+surface straddling `z = 0`, filtered rect occluding a cube behind it, anisotropic `rx ≠ ry` ellipsoid and cylinder silhouettes). After an intentional rendering change, regenerate the goldens with `SVG3_UPDATE_SNAPSHOTS=1 cargo test -p svg3 --test snapshot` and review the updated PNGs in the diff. On a mismatch the test writes the actual render as `<name>.actual.png` (git-ignored) next to the golden for inspection. `svg3/tests/filter.rs` complements the goldens with focused pixel-probe assertions per primitive — useful for fast feedback during a primitive's GPU-shader work, since each test only inspects a handful of pixels and rarely needs a regeneration step. `svg3/tests/wpt_filter_cases.rs` tracks migrated SVG filter WPT coverage with full coverage of every original WPT entry — cases whose target spec section is not yet implemented validate svg3's documented approximation (e.g. `BackgroundImage` falling back to `SourceGraphic` until `enable-background` capture lands). The approximations will tighten as the mask / enable-background subsystems land, without changing the test surface.

## CI / supply chain

- **GitHub Actions are pinned to full 40-char commit SHAs, never tags or branches.** When adding or bumping an action, resolve the release tag to its commit SHA (e.g. `gh api repos/<owner>/<repo>/commits/<tag> --jq .sha`) and pin that, with a trailing `# vX.Y.Z` comment for readability.
- CI is split between two runners. `svg3::render` rasterises with wgpu and its tests need a real GPU adapter, which the `ubuntu-latest` runner lacks — so lint, tests, and coverage run on macOS (Metal), and the Linux runner is kept only for the CodSpeed benchmark job:
  - **`macos-latest`** (`macos` job): `cargo fmt --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo llvm-cov --workspace --exclude app-macos --all-features --lcov --output-path lcov.info` (runs the tests under coverage instrumentation — the headless `svg3::render` render test exercises Metal here), `codecov/codecov-action` upload, `cargo build -p app-macos` + `cargo test -p app-macos`, and `cargo bench -p svg3 --bench render -- --sample-size 10` as a macOS/GPU full-renderer timing smoke check.
  - **`ubuntu-latest`** (`linux` job): `cargo codspeed build --workspace --exclude app-macos` + the `CodSpeedHQ/action` in `mode: simulation`. CodSpeed's simulation mode uses Valgrind (Linux-only), so the benches must run here; they are pure CPU and need no GPU.
- **Coverage:** `cargo llvm-cov` produces `lcov.info`, uploaded to Codecov by `codecov/codecov-action`. The project threshold is 3% (see `codecov.yml`); patch coverage is informational only. `app-macos` is excluded — its windowed event loop is not unit-testable, and counting it would create a permanent 0% drag. `fail_ci_if_error: false` so a missing/broken Codecov token does not break CI; add a `CODECOV_TOKEN` repo secret if uploads need to be reliable on private mirrors.

## Repository structure

- `svg3/`: the library. Three top-level submodules — `dom`, `style`, `render` — replacing what used to be three separate crates. Pure Rust outside of `render`, which depends on `wgpu`.
  - `svg3::dom` (`src/dom/`): runtime svg3 XML parsing (`quick-xml`) and the mutable element tree. svg3 is specified as an extension to SVG 1.1 ([`SPEC.md`](SPEC.md)) — the document root is `<svg>` and the parser recognises `<g>` (SVG 1.1 grouping), `<defs>`, `<rect>`, `<circle>`, `<ellipse>`, `<polygon>`, `<polyline>`, `<line>`, and `<path>` (SVG 1.1 shapes), `<filter>` plus every supported filter primitive (`<feGaussianBlur>`, `<feImage>`, `<feColorMatrix>`, `<feTurbulence>`, `<feSpecularLighting>`, `<feDiffuseLighting>`, `<feMorphology>`, `<feFlood>`, `<feDropShadow>`, `<feDisplacementMap>`, `<feConvolveMatrix>`, `<feComponentTransfer>`, `<feOffset>`, `<feMerge>` + `<feMergeNode>`, `<feBlend>`, `<feComposite>`, `<feTile>`) and their child elements (`<feFuncR/G/B/A>`, `<feDistantLight>`, `<fePointLight>`, `<feSpotLight>`), `<marker>` (SVG 1.1 markers), `<clipPath>` (referenced via `clip-path="url(#id)"`), paint servers (`<linearGradient>`, `<radialGradient>`, `<stop>`, `<pattern>`), the no-op stubs `<mask>` and `<foreignObject>` (parsed as definition/empty containers until their subsystems land), and `<cube>`, `<ellipsoid>`, `<cylinder>`, `<surface>` (svg3 3D primitives); other SVG 1.1 elements round-trip as `ElementKind::Unknown` until they are specialised. Split into `element.rs` (the `Document` / `Node` / `Element` / `ElementKind` data model) and `parser.rs` (the `quick-xml`-driven parser); `mod.rs` re-exports the public surface and owns the `ParseError` type.
  - `svg3::style` (`src/style.rs`): runtime CSS parsing + style resolution via [Stylo](https://crates.io/crates/stylo). The planned integration implements Stylo's `TElement`/`TNode`/`TDocument` traits over `svg3::dom`; **[Blitz (`blitz-dom`)](https://github.com/DioxusLabs/blitz) is the canonical reference** for driving Stylo over a custom DOM. Currently a skeleton.
  - `svg3::render` (`src/render/`): turns a parsed scene into GPU geometry with [wgpu](https://crates.io/crates/wgpu) — `build_scene` tessellates fills and strokes for `<rect>`, `<circle>`, `<ellipse>`, `<polygon>`, `<polyline>`, and `<path>`, plus stroke-only `<line>` geometry and svg3's 3D `<cube>`, `<ellipsoid>`, `<cylinder>`, and `<surface>` primitives, into a `Mesh`, including inherited `<g>` / root presentation attributes, composed `transform` attributes, stroke caps, joins, dashes, `pathLength` dash calibration, opacity attributes, referenced markers, and referenced paint servers (`<linearGradient>`, `<radialGradient>`, and rectangular-child `<pattern>` tiles via `<stop>` / child paints). `Renderer::encode_document` is the single GPU entry point both render paths share: it walks a parsed document, tessellates every supported shape, resolves paint-server references in `fill` / `stroke`, resolves `filter="url(#id)"` references to `<filter>` definitions parsed as an ordered chain of `<fe…>` primitives — `feGaussianBlur`, `feImage` (honors `preserveAspectRatio`), `feColorMatrix`, `feTurbulence`, `feSpecularLighting`, `feDiffuseLighting`, `feMorphology`, `feFlood`, `feDropShadow`, `feDisplacementMap`, `feConvolveMatrix` (honors `edgeMode`), `feComponentTransfer`, `feOffset`, `feMerge` (multi-input painter-order composite), `feBlend` (normal / multiply / screen / darken / lighten), `feComposite` (Porter-Duff over / in / out / atop / xor + arithmetic mode), `feTile` — renders filtered subtrees into an offscreen GPU `source` texture, runs the chain through ping/pong GPU passes (`shaders/filter.wgsl` for filter primitives and `shaders/image.wgsl` for decoded PNG data-URL `feImage`), and composites the result back in painter order into a caller-owned `wgpu::TextureView` through a caller-owned `wgpu::CommandEncoder`. Per SVG 1.1 §15.4, an unresolved `filter="url(#…)"` or a `<filter>` with no supported primitives renders the filtered element as transparent black; `<filter>` may also chain via `href` / `xlink:href` to inherit another filter's primitive list, with cycle-safe resolution (an `a → b → a` loop yields the same transparent-black result as a missing reference). Per-primitive `x/y/width/height` subregions clip each primitive's output to its authored rect, and `clip-path="url(#id)"` referencing a `<clipPath>` with a `<rect>` child applies before the filter chain (SVG 2 render order). `currentColor` resolves on `flood-color` / `lighting-color` from the filtered element's `color` attribute. `<feImage>` PNGs are decoded/uploaded lazily when a referenced filter paints; there is no eager decode step while parsing. `Renderer::render_to_image` wraps it with an offscreen sRGB texture, the shared `clear_target` helper, and CPU readback to produce an `Image`; `app-macos` wraps it with surface acquisition, `clear_target` to its background colour, and `frame.present()`. The lower-level `Renderer::create_scene` + `Renderer::draw` pair remains for callers that want to drive a pre-tessellated `Mesh` directly.

    `svg3::render` is organised into:
    - `mesh.rs`: the `Vertex`/`Mesh` geometry model plus GPU paint-server payloads.
    - `paint.rs`: paint-server definition / reference resolution (`<linearGradient>`, `<radialGradient>`, `<pattern>`).
    - `camera.rs`: `RenderConfig` and the 3D `Camera`.
    - `transform.rs`: SVG `transform` attribute parser (a v0 subset of SPEC §4.2) used by the scene walker and `<surface>` paths.
    - `shapes/`: per-shape geometry resolution and tessellation, one submodule per element kind (`rect.rs`, `circle.rs`, `ellipse.rs`, `polygon.rs`, `polyline.rs`, `line.rs`, `path.rs`, `cube.rs`, `ellipsoid.rs`, `cylinder.rs`, `surface.rs`, plus `triangulate.rs`); `mod.rs` carries shared length/colour parsing and the `Viewport` that percentages resolve against. Stroke parsing/tessellation lives under `shapes/stroke/` (`mod.rs` for the entry points, `dash.rs` for `stroke-dasharray` resolution + `pathLength` calibration, `markers.rs` for the start/mid/end placement enumerator).
    - `filters/`: SVG filter resolution. `mod.rs` carries the primitive types, `FilterDefinitions` / `FilterResolution`, and the per-`<fe…>` attribute parsers; `clip.rs` is the `<clipPath>` resolver (separate concern because clip-path runs before the filter chain in SVG 2 render order).
    - `scene/`: the document walk. `mod.rs` is `build_scene` / `build_render_plan` and the document → render-op planner; `markers.rs` is `<marker>` resolution + per-placement instancing.
    - `shaders/`: WGSL sources (`shader.wgsl` for the unified shape pipeline, `filter.wgsl` for every filter primitive, `image.wgsl` for `feImage`).
    - `gpu/`: the wgpu layer. `mod.rs` re-exports the public surface; `renderer.rs` owns the `Renderer` and `encode_document`; `pipeline.rs` builds every render pipeline and bind-group layout up front; `uniforms.rs` packs the WGSL-aligned uniform structs; `clear.rs` / `device.rs` / `image_decode.rs` / `readback.rs` are small utilities (target clears, adapter acquisition, `feImage` PNG decoding, headless CPU readback).

    `<cube>` (`shapes/cube.rs`) tessellates to a 6-face / 12-triangle solid mesh wound counter-clockwise from outside, applying SPEC §5.2's `size`/per-axis defaulting and skipping zero or negative dimensions. `<ellipsoid>` (`shapes/ellipsoid.rs`) tessellates the implicit surface `((x-cx)/rx)² + ((y-cy)/ry)² + ((z-cz)/rz)² ≤ 1` as a UV-parameterised mesh (16 latitude bands × 32 longitude segments) wound outward CCW, with SPEC §5.3's `r`/per-axis defaulting matching `<cube>`'s `size` rule. `<cylinder>` (`shapes/cylinder.rs`) tessellates a right elliptical cylinder aligned to Z as two caps plus a segmented side wall, applying SPEC §5.5's `r`/`rx`/`ry`/`depth` defaulting. `<surface>` (`shapes/surface.rs`) consumes a sequence of `<path>` children plus its own SVG-path-data-like `d` attribute (`M`/`L`/`Q`/`C`/`Z` over child-path indices, see SPEC §5.4) and tessellates a chain of degree-(N-1) Bezier patches into an S×M grid via de Casteljau; per-child `transform` attributes (parsed by the `transform` module) lift each cross-section into 3D, and the scene walker composes the surface's own transform plus ancestor group transforms. The scene walker's `owns_children` predicate suppresses generic re-dispatch of `<surface>`'s `<path>` children so they don't double-render at `z = 0`. All four 3D primitives leave the `z = 0` plane, so the default orthographic projection's depth range is widened to ±100_000 user units (matching the perspective camera's far plane) so practical 3D depths are not clipped. Root `<svg width>` / `<svg height>` define the percentage viewport, falling back to the render target when omitted. An optional `Camera` on `RenderConfig` views the scene — including 2D content in the `z = 0` plane and 3D content above and below it — through a movable 3D perspective camera; with no camera set, content is drawn flat through the orthographic default (a cube collapses to its axis-aligned bounding rectangle).

  **Depth-aware 2D ↔ 3D occlusion (SPEC §7.3).** Rendering goes through one unified shape pipeline with `depth_compare: LessEqual` + depth write enabled. To avoid coplanar z-fighting between 2D shapes under perspective (perspective-correct interpolation produces tiny per-triangle NDC depth drift even on the same world plane), `scene::build_render_plan` shifts each 2D shape forward in `+Z` by a small per-shape `Z_PAINTER_STRIDE` (currently `0.1` user units, ~10× the float precision floor at the default camera distance). 3D shapes keep their authored world Z so spatial occlusion vs the 2D plane and other 3D content works per SPEC §7.3. Within the fragment shader, SDF shapes `discard` when their signed distance exceeds the AA pad so transparent corners of the bounding quad don't spuriously occupy depth. The filter composite participates in depth too: its fragment shader samples the filter source's depth texture (allocated with `TEXTURE_BINDING` usage) and writes it as `@builtin(frag_depth)`, so filtered 2D content correctly occludes 3D behind it and a cube in front of `z = 0` paints over a later filtered rect. Halo pixels (where the source depth is the cleared far value) fall back to a `FILTER_PLANE_NDC_DEPTH = 0.5` sentinel — exact for the orthographic default, an approximation for perspective (per SPEC §6.3 filter on a 3D element is implementation-defined anyway). The public `Renderer::draw` API now requires a render pass with a depth attachment matching `DEPTH_FORMAT` (`Depth32Float`); the constant is re-exported so windowed callers can allocate a matching depth texture. `encode_document` allocates a fresh depth texture per encode (and one per filter chain for the offscreen filter source), so the windowed `encode_document` path is unchanged. The Stylo-driven styled-scene path is still to come; `app-macos` drives this same GPU path into a windowed Metal surface.
- `app-macos/`: native macOS demo binary (`svg3-macos`). A winit 0.30 event loop driving a wgpu (Metal) surface; it prompts for an SVG string on launch (and again via Command+O/Command+I), parses it with `svg3::dom`, and per frame drives `svg3::render::Renderer::encode_document` against the swap-chain texture — the same GPU path the headless renderer uses, so supported filled/stroked `<rect>` / `<circle>` / `<ellipse>` / `<polygon>` / `<polyline>` / `<path>` content, stroked `<line>` content, referenced markers, and referenced `<feGaussianBlur>` / PNG data-URL `<feImage>` filters render identically in both. The document is viewed through a movable orbit camera (`camera.rs`) — left-drag or the arrow keys orbit, the scroll wheel zooms, `R` reframes head-on. Plus `app-macos/macos/Info.plist` and `scripts/bundle-macos.sh` for assembling a `.app`.

## Project design overview

The document language is specified in [`SPEC.md`](SPEC.md) (editor's draft). `SPEC.md` is a normative extension spec — it defines `svg3` as an extension to SVG 1.1 that adds three-dimensional graphics elements (`<cube>`, `<ellipsoid>`, `<surface>`, `<cylinder>`), 3D transform functions, and a rendering model for them. It does **not** carry per-section implementation status; what the current scaffold actually implements is tracked in [`README.md`](README.md)'s Roadmap and in the issue tracker. When the spec and the implementation diverge, that is a bug worth filing, not a status the spec records.

- A document-driven 3D renderer: SVG/XML extended with 3D elements.
- **Runtime parsing.** Documents are parsed at runtime: XML via `quick-xml` (in `svg3::dom`), CSS via Stylo's parser (`cssparser`/`selectors`, in `svg3::style`). svg3 deliberately does **not** replicate the Paws template's compile-time style preprocessor (`view-macros`' `css!()` macro + `paws-style-ir`). There is no proc-macro / preprocessor crate; do not add one.
- **Stylo** provides web-standard CSS behavior and computed-style resolution.
- **wgpu** provides cross-platform native GPU rendering (Metal/Vulkan/DX12).
- **Native only — no WASM.** Targets native platforms (macOS first, then Windows/Linux). There is no WASM engine, `wasm32` target, or browser/WebGPU path planned. Deliberate divergence from the Paws template's `wasmtime-engine` — do not add one.
- The core (`svg3::dom`/`svg3::style`/`svg3::render`/`svg3`) is platform-agnostic. Platform/demo glue lives in `app-macos`, which does real work (a winit 0.30 event loop + a wgpu Metal surface that renders pasted supported SVG 2D shapes). The no-placeholder rule still applies to any **future** app crate (e.g. Windows/Linux demos): do not add a stub app crate before it does real work.

## How to run

```sh
cargo build --workspace
cargo test  --workspace
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings

cargo run -p app-macos                        # native macOS demo; paste SVG text into the dialog
bash scripts/bundle-macos.sh                  # assemble target/release/bundle/svg3-macos.app

cargo bench --workspace                       # criterion benches (codspeed-instrumented)
cargo codspeed build                          # build the CodSpeed-instrumented bench binaries
```

## Benchmarking

Benchmarks live in each crate's `benches/` directory and use
[`codspeed-criterion-compat`](https://docs.codspeed.io/benchmarks/rust/criterion) —
the criterion API, instrumented for CodSpeed. It is declared in the root
`[workspace.dependencies]` as `criterion = { package = "codspeed-criterion-compat", ... }`,
so a bench file just writes `use criterion::*;` and both `cargo bench` and
`cargo codspeed build` produce the right binary.

- **Run locally:** `cargo bench --workspace` (or `-p svg3` / `-p svg3`).
- **Build the instrumented binaries:** `cargo codspeed build` (install
  the cargo subcommand once with `cargo install cargo-codspeed`).
- **CI:** the `linux` job in `.github/workflows/ci.yml` runs `cargo
  codspeed build --workspace --exclude app-macos` and then
  `CodSpeedHQ/action@…` in `mode: simulation` on `ubuntu-latest` (the
  simulation mode uses Valgrind, which is Linux-only). CodSpeed posts
  per-benchmark deltas on PRs.

Only crates with real work to measure ship benches. Today that is
`svg3::dom`'s `parse` (XML parsing), `svg3::render`'s `tessellate`
(`<rect>`, `<circle>`, `<ellipse>`, `<polygon>`, filled `<polyline>`,
`<line>`, and `<path>` tessellation via `build_scene`), and
`svg3::render`'s `render` full-renderer bench (`Renderer::render_to_image`).
The full-renderer bench needs a GPU adapter and self-skips when none is
available. The style crate is still a skeleton, so benches for it are out
of scope until the Stylo cascade lands.

## Maintaining this file

On every change, assess whether `AGENTS.md` needs an update and update it when needed. After finishing work, verify it is still accurate.
