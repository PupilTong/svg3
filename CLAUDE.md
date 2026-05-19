# Agent / Contributor Instructions

This repository supports LLM-based assistants. The working language is English.

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
- CI runs on `macos-latest` (the first target platform): `cargo fmt --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test --workspace`.

## Repository structure

- `svg3-dom/`: runtime SVG3 XML parsing (`quick-xml`) and the mutable element tree (the 3D element model: `<scene>`, `<cube>`, `<ellipsoid>`, …). Pure Rust, no GPU dependencies.
- `svg3-style/`: runtime CSS parsing + style resolution via [Stylo](https://crates.io/crates/stylo). The planned integration implements Stylo's `TElement`/`TNode`/`TDocument` traits over `svg3-dom`; **[Blitz (`blitz-dom`)](https://github.com/DioxusLabs/blitz) is the canonical reference** for driving Stylo over a custom DOM. Currently a skeleton.
- `svg3-render/`: Turns a styled scene into GPU draw calls with [wgpu](https://crates.io/crates/wgpu). Currently a skeleton.
- `svg3/`: Umbrella crate. Re-exports the layers and exposes the public `parse → render` facade.
- `app-macos/`: Desktop demo binary (`winit` + `wgpu`), macOS first. Currently a stub `main` (no window loop yet).

## Project design overview

- A document-driven 3D renderer: SVG/XML extended with 3D elements.
- **Runtime parsing.** Documents are parsed at runtime: XML via `quick-xml` (in `svg3-dom`), CSS via Stylo's parser (`cssparser`/`selectors`, in `svg3-style`). svg3 deliberately does **not** replicate the Paws template's compile-time style preprocessor (`view-macros`' `css!()` macro + `paws-style-ir`). There is no proc-macro / preprocessor crate; do not add one.
- **Stylo** provides web-standard CSS behavior and computed-style resolution.
- **wgpu** provides cross-platform native GPU rendering (Metal/Vulkan/DX12).
- **Native only — no WASM.** Targets native platforms (macOS first, then Windows/Linux). There is no WASM engine, `wasm32` target, or browser/WebGPU path planned. Deliberate divergence from the Paws template's `wasmtime-engine` — do not add one.
- The core (`svg3-dom`/`svg3-style`/`svg3-render`/`svg3`) is platform-agnostic; platform glue lives in app crates (`app-macos`, future `app-*`).

## How to run

```sh
cargo build --workspace
cargo test  --workspace
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo run -p app-macos
```

## Maintaining this file

On every change, assess whether `CLAUDE.md` needs an update and update it when needed. After finishing work, verify it is still accurate.
