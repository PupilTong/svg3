# Agent / Contributor Instructions

This repository supports LLM-based assistants. The working language is English.

`AGENTS.md` is the single canonical agent-instructions file, shared across tools (Claude Code and GPT/Codex both read it). There is no `CLAUDE.md` — edit this file instead.

## General guidelines

- **Rebase onto `origin/main` before starting any work** to avoid merge conflicts and stale base commits.
- Keep changes minimal and focused.
- Follow existing style conventions and formatting.
- Rust **2021** edition; the toolchain is pinned by [`rust-toolchain.toml`](rust-toolchain.toml) (`nightly-2026-04-20`). `rustup` selects it automatically in this repo.
- Use cargo workspace conventions (`*.workspace = true` for shared metadata and dependencies; declare versions once in the root `[workspace.dependencies]`).
- Update or add tests when relevant. Always run tests.
- **Test behavior, not implementation.** Prefer end-to-end paths (parse a document, run it through the pipeline, assert on output) over asserting on internal helpers or intermediate structures.
- Error handling returns specific typed errors (`thiserror`); avoid stringly-typed errors.
- All `unsafe` blocks must have a `// SAFETY:` comment explaining the upheld invariant.
- Prefer `pub(crate)` or private by default; only make items `pub` when they are part of a crate's public API.

## Formatting

- CI runs `cargo fmt --check` using the toolchain pinned in `rust-toolchain.toml`, which is the same toolchain `rustup` selects locally — so local and CI formatting agree as long as you don't override the toolchain.
- Always run `cargo fmt --check` (not just `cargo fmt`) before committing.

## Testing

- `cargo test --workspace` runs the unit and integration tests. The GPU-backed tests in `svg3-render` — the headless render test and the `tests/snapshot.rs` E2E suite — self-skip when no GPU adapter is available, so they run on macOS but not on a GPU-less host.
- **Snapshot tests:** `svg3-render/tests/snapshot.rs` renders filled/stroked `<rect>`, `<circle>`, `<ellipse>`, `<polygon>`, `<polyline>`, stroked `<line>`, filled/stroked `<path>`, referenced `<marker>` documents, and the full referenced-filter set headlessly and compares them against the committed golden PNGs in `svg3-render/tests/snapshots/` (the reviewable images). The filter coverage spans every supported primitive — `feGaussianBlur` (multiple axis configurations and source shapes), PNG data-URL `feImage` (explicit / intrinsic / percentage sizing, `href` and `xlink:href`, alpha, blur, painter order, and skipped unsupported / invalid cases), `feColorMatrix` (saturate / luminanceToAlpha / hueRotate), `feTurbulence` (turbulence and fractalNoise), `feSpecularLighting`, `feDiffuseLighting`, `feMorphology` (dilate / erode), `feFlood`, `feDropShadow`, `feDisplacementMap`, `feConvolveMatrix`, `feComponentTransfer`, plus chained primitive cases. After an intentional rendering change, regenerate the goldens with `SVG3_UPDATE_SNAPSHOTS=1 cargo test -p svg3-render --test snapshot` and review the updated PNGs in the diff. On a mismatch the test writes the actual render as `<name>.actual.png` (git-ignored) next to the golden for inspection. `svg3-render/tests/filter.rs` complements the goldens with focused pixel-probe assertions per primitive — useful for fast feedback during a primitive's GPU-shader work, since each test only inspects a handful of pixels and rarely needs a regeneration step.

## CI / supply chain

- **GitHub Actions are pinned to full 40-char commit SHAs, never tags or branches.** When adding or bumping an action, resolve the release tag to its commit SHA (e.g. `gh api repos/<owner>/<repo>/commits/<tag> --jq .sha`) and pin that, with a trailing `# vX.Y.Z` comment for readability.
- CI is split between two runners. `svg3-render` rasterises with wgpu and its tests need a real GPU adapter, which the `ubuntu-latest` runner lacks — so lint, tests, and coverage run on macOS (Metal), and the Linux runner is kept only for the CodSpeed benchmark job:
  - **`macos-latest`** (`macos` job): `cargo fmt --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo llvm-cov --workspace --exclude app-macos --all-features --lcov --output-path lcov.info` (runs the tests under coverage instrumentation — the headless `svg3-render` render test exercises Metal here), `codecov/codecov-action` upload, `cargo build -p app-macos` + `cargo test -p app-macos`, and `cargo bench -p svg3-render --bench render -- --sample-size 10` as a macOS/GPU full-renderer timing smoke check.
  - **`ubuntu-latest`** (`linux` job): `cargo codspeed build --workspace --exclude app-macos` + the `CodSpeedHQ/action` in `mode: simulation`. CodSpeed's simulation mode uses Valgrind (Linux-only), so the benches must run here; they are pure CPU and need no GPU.
- **Coverage:** `cargo llvm-cov` produces `lcov.info`, uploaded to Codecov by `codecov/codecov-action`. The project threshold is 3% (see `codecov.yml`); patch coverage is informational only. `app-macos` is excluded — its windowed event loop is not unit-testable, and counting it would create a permanent 0% drag. `fail_ci_if_error: false` so a missing/broken Codecov token does not break CI; add a `CODECOV_TOKEN` repo secret if uploads need to be reliable on private mirrors.

## Repository structure

- `svg3-dom/`: runtime svg3 XML parsing (`quick-xml`) and the mutable element tree. svg3 is specified as an extension to SVG 1.1 ([`SPEC.md`](SPEC.md)) — the document root is `<svg>` and the parser recognises `<g>` (SVG 1.1 grouping), `<defs>`, `<rect>`, `<circle>`, `<ellipse>`, `<polygon>`, `<polyline>`, `<line>`, and `<path>` (SVG 1.1 shapes), `<filter>` plus every supported filter primitive (`<feGaussianBlur>`, `<feImage>`, `<feColorMatrix>`, `<feTurbulence>`, `<feSpecularLighting>`, `<feDiffuseLighting>`, `<feMorphology>`, `<feFlood>`, `<feDropShadow>`, `<feDisplacementMap>`, `<feConvolveMatrix>`, `<feComponentTransfer>`) and their child elements (`<feFuncR/G/B/A>`, `<feDistantLight>`, `<fePointLight>`, `<feSpotLight>`), `<marker>` (SVG 1.1 markers), `<cube>`, and `<ellipsoid>` (svg3 3D primitives); other SVG 1.1 elements round-trip as `ElementKind::Unknown` until they are specialised. Pure Rust, no GPU dependencies.
- `svg3-style/`: runtime CSS parsing + style resolution via [Stylo](https://crates.io/crates/stylo). The planned integration implements Stylo's `TElement`/`TNode`/`TDocument` traits over `svg3-dom`; **[Blitz (`blitz-dom`)](https://github.com/DioxusLabs/blitz) is the canonical reference** for driving Stylo over a custom DOM. Currently a skeleton.
- `svg3-render/`: turns a parsed scene into GPU geometry with [wgpu](https://crates.io/crates/wgpu) — `build_scene` tessellates fills and strokes for `<rect>`, `<circle>`, `<ellipse>`, `<polygon>`, `<polyline>`, and `<path>`, plus stroke-only `<line>` geometry, into a `Mesh`, including stroke caps, joins, dashes, `pathLength` dash calibration, opacity attributes, and referenced markers. `Renderer::encode_document` is the single GPU entry point both render paths share: it walks a parsed document, tessellates every supported shape, resolves `filter="url(#id)"` references to `<filter>` definitions parsed as an ordered chain of `<fe…>` primitives — `feGaussianBlur`, `feImage`, `feColorMatrix`, `feTurbulence`, `feSpecularLighting`, `feDiffuseLighting`, `feMorphology`, `feFlood`, `feDropShadow`, `feDisplacementMap`, `feConvolveMatrix`, `feComponentTransfer` — renders filtered subtrees into an offscreen GPU `source` texture, runs the chain through ping/pong GPU passes (`filter.wgsl` for filter primitives and `image.wgsl` for decoded PNG data-URL `feImage`), and composites the result back in painter order into a caller-owned `wgpu::TextureView` through a caller-owned `wgpu::CommandEncoder`. `<feImage>` PNGs are decoded/uploaded lazily when a referenced filter paints; there is no eager decode step while parsing. `Renderer::render_to_image` wraps it with an offscreen sRGB texture, the shared `clear_target` helper, and CPU readback to produce an `Image`; `app-macos` wraps it with surface acquisition, `clear_target` to its background colour, and `frame.present()`. The lower-level `Renderer::create_scene` + `Renderer::draw` pair remains for callers that want to drive a pre-tessellated `Mesh` directly. The crate is organised into focused modules: `mesh` (the `Vertex`/`Mesh` geometry model), `shapes` (per-shape geometry resolution and tessellation, one submodule per shape under `shapes/`, with shared stroke parsing/tessellation in `shapes/stroke.rs`), `filters` (supported filter definition/reference resolution), `scene` (`build_scene` plus the document render plan and marker instancing), `camera` (`RenderConfig` and the 3D `Camera`), and `renderer` (the wgpu `Renderer`). Length parsing (absolute and percentage) for length-based shapes, `fill`/`stroke` paint resolution, and the `Viewport` that percentages resolve against live in `shapes.rs`; `<polygon>` and `<polyline>` numeric `points` parsing lives in their shape modules, while `<path>` uses `svgtypes` for `d` parsing and Lyon for fill/stroke tessellation. Root `<svg width>` / `<svg height>` define the percentage viewport, falling back to the render target when omitted. An optional `Camera` on `RenderConfig` views the scene — including 2D content in the `z = 0` plane — through a movable 3D perspective camera; with no camera set, content is drawn flat through the orthographic default. The Stylo-driven styled-scene path is still to come; `app-macos` drives this same GPU path into a windowed Metal surface.
- `svg3/`: Umbrella crate. Re-exports the layers and exposes the public `parse → render` facade.
- `app-macos/`: native macOS demo binary (`svg3-macos`). A winit 0.30 event loop driving a wgpu (Metal) surface; it prompts for an SVG string on launch (and again via Command+O/Command+I), parses it with `svg3-dom`, and per frame drives `Renderer::encode_document` against the swap-chain texture — the same GPU path the headless renderer uses, so supported filled/stroked `<rect>` / `<circle>` / `<ellipse>` / `<polygon>` / `<polyline>` / `<path>` content, stroked `<line>` content, referenced markers, and referenced `<feGaussianBlur>` / PNG data-URL `<feImage>` filters render identically in both. The document is viewed through a movable orbit camera (`camera.rs`) — left-drag or the arrow keys orbit, the scroll wheel zooms, `R` reframes head-on. Plus `app-macos/macos/Info.plist` and `scripts/bundle-macos.sh` for assembling a `.app`.

## Project design overview

The document language is specified in [`SPEC.md`](SPEC.md) (editor's draft). `SPEC.md` is a normative extension spec — it defines `svg3` as an extension to SVG 1.1 that adds three-dimensional graphics elements (`<cube>`, `<ellipsoid>`), 3D transform functions, and a rendering model for them. It does **not** carry per-section implementation status; what the current scaffold actually implements is tracked in [`README.md`](README.md)'s Roadmap and in the issue tracker. When the spec and the implementation diverge, that is a bug worth filing, not a status the spec records.

- A document-driven 3D renderer: SVG/XML extended with 3D elements.
- **Runtime parsing.** Documents are parsed at runtime: XML via `quick-xml` (in `svg3-dom`), CSS via Stylo's parser (`cssparser`/`selectors`, in `svg3-style`). svg3 deliberately does **not** replicate the Paws template's compile-time style preprocessor (`view-macros`' `css!()` macro + `paws-style-ir`). There is no proc-macro / preprocessor crate; do not add one.
- **Stylo** provides web-standard CSS behavior and computed-style resolution.
- **wgpu** provides cross-platform native GPU rendering (Metal/Vulkan/DX12).
- **Native only — no WASM.** Targets native platforms (macOS first, then Windows/Linux). There is no WASM engine, `wasm32` target, or browser/WebGPU path planned. Deliberate divergence from the Paws template's `wasmtime-engine` — do not add one.
- The core (`svg3-dom`/`svg3-style`/`svg3-render`/`svg3`) is platform-agnostic. Platform/demo glue lives in `app-macos`, which does real work (a winit 0.30 event loop + a wgpu Metal surface that renders pasted supported SVG 2D shapes). The no-placeholder rule still applies to any **future** app crate (e.g. Windows/Linux demos): do not add a stub app crate before it does real work.

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

- **Run locally:** `cargo bench --workspace` (or `-p svg3-dom` / `-p svg3-render`).
- **Build the instrumented binaries:** `cargo codspeed build` (install
  the cargo subcommand once with `cargo install cargo-codspeed`).
- **CI:** the `linux` job in `.github/workflows/ci.yml` runs `cargo
  codspeed build --workspace --exclude app-macos` and then
  `CodSpeedHQ/action@…` in `mode: simulation` on `ubuntu-latest` (the
  simulation mode uses Valgrind, which is Linux-only). CodSpeed posts
  per-benchmark deltas on PRs.

Only crates with real work to measure ship benches. Today that is
`svg3-dom`'s `parse` (XML parsing), `svg3-render`'s `tessellate`
(`<rect>`, `<circle>`, `<ellipse>`, `<polygon>`, filled `<polyline>`,
`<line>`, and `<path>` tessellation via `build_scene`), and
`svg3-render`'s `render` full-renderer bench (`Renderer::render_to_image`).
The full-renderer bench needs a GPU adapter and self-skips when none is
available. The style crate is still a skeleton, so benches for it are out
of scope until the Stylo cascade lands.

## Maintaining this file

On every change, assess whether `AGENTS.md` needs an update and update it when needed. After finishing work, verify it is still accurate.
