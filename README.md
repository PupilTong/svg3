# svg3

**An extended SVG renderer for 3D models.**

`svg3` is an extension to SVG 1.1: it inherits SVG 1.1's `<svg>` root and 2D
drawing model and adds three-dimensional graphics elements (`<cube>`,
`<ellipsoid>`, …) plus 3D transform functions, rendered on the GPU. Styling
stays faithful to the web platform: element styles are resolved with
[Stylo](https://crates.io/crates/stylo) (Servo's CSS engine), and the
resulting scene is painted with [wgpu](https://crates.io/crates/wgpu).

The project is built from the ground up on the current Rust graphics ecosystem.
Demos will target multiple platforms; a native macOS desktop app is the first.

The document language is specified in [SPEC.md](SPEC.md) (editor's draft).

## Pipeline

```
svg3 XML   ──►  svg3-dom    ──►  svg3-style   ──►  svg3-render  ──►  GPU surface
(text)          (element tree)   (Stylo cascade)   (wgpu draw)
```

## Workspace

| Crate         | Responsibility                                                       |
|---------------|----------------------------------------------------------------------|
| `svg3-dom`    | Parse svg3 XML into a mutable element tree (SVG-1.1 root + 3D extensions). |
| `svg3-style`  | Resolve computed styles for the element tree via Stylo.              |
| `svg3-render` | Turn a styled scene into GPU draw calls with wgpu.                    |
| `svg3`        | Umbrella crate: public API tying DOM + style + render together.      |
| `app-macos`   | Native macOS demo: a winit window with a wgpu (Metal) surface.       |

> **Status: early scaffolding.** `app-macos` opens a real Metal-backed Cocoa
> window and clears it to a solid colour each frame (roadmap item 1).
> `svg3-dom` parses svg3 XML into an element tree with raw attributes
> (roadmap item 2) — currently the `<svg>` root, `<g>`, `<rect>`,
> `<circle>`, `<cube>`, and `<ellipsoid>` are recognised; the rest of
> SVG 1.1 round-trips as unknown elements. `svg3-render` tessellates
> `<rect>` and `<circle>` and rasterises them headlessly to an image
> (roadmap item 3). The Stylo cascade is still a skeleton — see the
> roadmap below.

## Toolchain

The Rust toolchain is pinned in [`rust-toolchain.toml`](rust-toolchain.toml)
(`nightly-2026-04-20`). `rustup` selects it automatically inside this repo.

## Build & run

```sh
cargo build --workspace          # first build is slow (stylo + wgpu)
cargo test  --workspace
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings

cargo run -p app-macos                       # open the native macOS window
bash scripts/bundle-macos.sh                 # build a .app: target/release/bundle/svg3-macos.app

cargo bench --workspace                      # criterion benches (codspeed-instrumented)
```

## Roadmap

1. **(done)** winit event loop + wgpu surface — a window that clears to a colour (the `app-macos` crate).
2. **(done)** svg3 XML parsing → element tree (`<svg>` root with `<g>`, `<cube>`, `<ellipsoid>`; per [SPEC.md](SPEC.md)).
3. **(done)** 2D basic shapes — tessellate `<rect>` and `<circle>` and render them headlessly to an image (`svg3-render`).
4. 3D primitive mesh generation; render a single `<cube>`.
5. Real Stylo integration (computed styles drive material/transform).
6. Multi-platform native demos (Windows, Linux) + Linux/Windows CI.
7. Native macOS `.app` bundling.

## License

[MPL-2.0](LICENSE).
