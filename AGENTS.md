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

## CI / supply chain

- **GitHub Actions are pinned to full 40-char commit SHAs, never tags or branches.** When adding or bumping an action, resolve the release tag to its commit SHA (e.g. `gh api repos/<owner>/<repo>/commits/<tag> --jq .sha`) and pin that, with a trailing `# vX.Y.Z` comment for readability.
- CI is split between two runners:
  - **`ubuntu-latest`** (`linux` job): `cargo fmt --check`, `cargo clippy --workspace --exclude app-macos --all-targets --all-features -- -D warnings`, `cargo llvm-cov --workspace --exclude app-macos --all-features --lcov --output-path lcov.info` (runs tests under coverage instrumentation), `codecov/codecov-action` upload, then `cargo codspeed build --workspace --exclude app-macos` + the `CodSpeedHQ/action` in `mode: simulation`. CodSpeed's simulation mode uses Valgrind (Linux-only), so benches must run here.
  - **`macos-latest`** (`macos` job): `cargo clippy -p app-macos`, `cargo build -p app-macos`, `cargo test -p app-macos`. The only crate that needs Apple frameworks is `app-macos` (winit → Cocoa, wgpu → Metal); the library crates compile fine on Linux and are checked there.
- **Coverage:** `cargo llvm-cov` produces `lcov.info`, uploaded to Codecov by `codecov/codecov-action`. The project threshold is 3% (see `codecov.yml`); patch coverage is informational only. `app-macos` is excluded — its windowed event loop is not unit-testable, and counting it would create a permanent 0% drag. `fail_ci_if_error: false` so a missing/broken Codecov token does not break CI; add a `CODECOV_TOKEN` repo secret if uploads need to be reliable on private mirrors.

## Repository structure

- `svg3-dom/`: runtime svg3 XML parsing (`quick-xml`) and the mutable element tree. svg3 is specified as an extension to SVG 1.1 ([`SPEC.md`](SPEC.md)) — the document root is `<svg>` and the v0 parser recognises `<g>` (SVG 1.1 grouping), `<cube>`, and `<ellipsoid>` (svg3 3D primitives); other SVG 1.1 elements round-trip as `ElementKind::Unknown` until they are specialised. Pure Rust, no GPU dependencies.
- `svg3-style/`: runtime CSS parsing + style resolution via [Stylo](https://crates.io/crates/stylo). The planned integration implements Stylo's `TElement`/`TNode`/`TDocument` traits over `svg3-dom`; **[Blitz (`blitz-dom`)](https://github.com/DioxusLabs/blitz) is the canonical reference** for driving Stylo over a custom DOM. Currently a skeleton.
- `svg3-render/`: Turns a styled scene into GPU draw calls with [wgpu](https://crates.io/crates/wgpu). Currently a skeleton.
- `svg3/`: Umbrella crate. Re-exports the layers and exposes the public `parse → render` facade.
- `app-macos/`: native macOS demo binary (`svg3-macos`). A winit 0.30 event loop driving a wgpu (Metal) surface that clears to a solid colour. Plus `app-macos/macos/Info.plist` and `scripts/bundle-macos.sh` for assembling a `.app`. Does **not** depend on `svg3` yet (the parse → render path is unimplemented).

## Project design overview

The document language is specified in [`SPEC.md`](SPEC.md) (editor's draft). `SPEC.md` is a normative extension spec — it defines `svg3` as an extension to SVG 1.1 that adds three-dimensional graphics elements (`<cube>`, `<ellipsoid>`), 3D transform functions, and a rendering model for them. It does **not** carry per-section implementation status; what the current scaffold actually implements is tracked in [`README.md`](README.md)'s Roadmap and in the issue tracker. When the spec and the implementation diverge, that is a bug worth filing, not a status the spec records.

- A document-driven 3D renderer: SVG/XML extended with 3D elements.
- **Runtime parsing.** Documents are parsed at runtime: XML via `quick-xml` (in `svg3-dom`), CSS via Stylo's parser (`cssparser`/`selectors`, in `svg3-style`). svg3 deliberately does **not** replicate the Paws template's compile-time style preprocessor (`view-macros`' `css!()` macro + `paws-style-ir`). There is no proc-macro / preprocessor crate; do not add one.
- **Stylo** provides web-standard CSS behavior and computed-style resolution.
- **wgpu** provides cross-platform native GPU rendering (Metal/Vulkan/DX12).
- **Native only — no WASM.** Targets native platforms (macOS first, then Windows/Linux). There is no WASM engine, `wasm32` target, or browser/WebGPU path planned. Deliberate divergence from the Paws template's `wasmtime-engine` — do not add one.
- The core (`svg3-dom`/`svg3-style`/`svg3-render`/`svg3`) is platform-agnostic. Platform/demo glue lives in `app-macos`, which does real work (a winit 0.30 event loop + a wgpu Metal surface that clears each frame). The no-placeholder rule still applies to any **future** app crate (e.g. Windows/Linux demos): do not add a stub app crate before it does real work.

## How to run

```sh
cargo build --workspace
cargo test  --workspace
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings

cargo run -p app-macos                        # native macOS window (clears to a colour)
bash scripts/bundle-macos.sh                  # assemble target/release/bundle/svg3-macos.app

cargo bench -p svg3-dom                       # criterion benches (codspeed-instrumented)
cargo codspeed build                          # build the CodSpeed-instrumented bench binaries
```

## Benchmarking

Benchmarks live in each crate's `benches/` directory and use
[`codspeed-criterion-compat`](https://docs.codspeed.io/benchmarks/rust/criterion) —
the criterion API, instrumented for CodSpeed. It is declared in the root
`[workspace.dependencies]` as `criterion = { package = "codspeed-criterion-compat", ... }`,
so a bench file just writes `use criterion::*;` and both `cargo bench` and
`cargo codspeed build` produce the right binary.

- **Run locally:** `cargo bench -p svg3-dom`.
- **Build the instrumented binaries:** `cargo codspeed build` (install
  the cargo subcommand once with `cargo install cargo-codspeed`).
- **CI:** the `linux` job in `.github/workflows/ci.yml` runs `cargo
  codspeed build --workspace --exclude app-macos` and then
  `CodSpeedHQ/action@…` in `mode: simulation` on `ubuntu-latest` (the
  simulation mode uses Valgrind, which is Linux-only). CodSpeed posts
  per-benchmark deltas on PRs.

Only crates with real work to measure ship benches. Today that is
`svg3-dom::parse` only — the style and render crates are still skeletons,
so benches for them are out of scope until the cascade and the GPU
pipeline land.

## Maintaining this file

On every change, assess whether `AGENTS.md` needs an update and update it when needed. After finishing work, verify it is still accurate.
