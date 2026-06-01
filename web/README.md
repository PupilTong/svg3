# svg3 · web

A browser front-end for [`svg3`](../README.md): paste an SVG / svg3 document and
watch it render **live on the GPU (WebGPU)**, with an orbit camera for 3D scenes
— the same experience as the native `app-macos` demo, in a web page.

All GPU rendering **and** the canvas drawing run **off the main thread in a Web
Worker**: the worker owns a transferred `OffscreenCanvas`, drives the
WebAssembly-compiled `svg3` renderer (`svg3-web`) on a wgpu device, reads each
rendered frame back, and paints it onto the canvas via a 2D context. The UI
thread only edits text and forwards pointer / keyboard input.

```
 main thread (UI)                       worker thread (render)
 ────────────────                       ──────────────────────
 textarea + <canvas> + controls         svg3-web::WebRenderer (wasm)
 transferControlToOffscreen() ─canvas─▶   └ wgpu device → offscreen sRGB texture
 pointer/keys/edits ──messages──────▶     └ encode_document → read pixels back
                                          putImageData → OffscreenCanvas (2D ctx)
```

## Requirements

- **A WebGPU browser.** Chrome / Edge 113+, Safari 18+, or Firefox with WebGPU
  enabled. Without `navigator.gpu` the app shows a "WebGPU required" panel and
  does not start the worker. (WebGPU needs **no** cross-origin isolation /
  COOP-COEP, so it deploys cleanly to GitHub Pages.)
- [pnpm](https://pnpm.io) and Node 20+.
- A Rust toolchain with the `wasm32-unknown-unknown` target and
  [`wasm-pack`](https://rustwasm.github.io/wasm-pack/):

  ```sh
  rustup target add wasm32-unknown-unknown
  cargo install wasm-pack            # or: https://rustwasm.github.io/wasm-pack/installer/
  ```

## Develop

```sh
cd web
pnpm install
pnpm wasm:dev      # wasm-pack build ../svg3-web --target web --out-dir ../web/src/wasm --dev
pnpm dev           # rsbuild dev server at http://localhost:3000
```

Re-run `pnpm wasm:dev` after changing any Rust (`svg3` or `svg3-web`); rsbuild
hot-reloads the TypeScript/CSS on its own.

## Build for GitHub Pages

```sh
pnpm wasm:build    # release wasm (+ wasm-opt)
PAGES=1 pnpm build # rsbuild → web/dist, with all assets under /svg3/
pnpm preview       # serve the production build locally
```

`PAGES=1` sets rsbuild's `output.assetPrefix` to `/svg3/` so the worker chunk
and the `svg3_web_bg.wasm` asset resolve under the project-site path. The
[`pages.yml`](../.github/workflows/pages.yml) workflow runs exactly this on a
push to `main` and publishes `web/dist` (enable Pages → Source = "GitHub
Actions" once in the repo settings).

## Layout

| Path | Role |
|---|---|
| `src/main.ts` | UI thread: WebGPU detection, `transferControlToOffscreen`, input → messages, status |
| `src/worker.ts` | Render worker: loads the wasm, owns `WebRenderer`, serializes render/mutation, blits frames |
| `src/protocol.ts` | The main ⇄ worker message protocol |
| `src/demo.ts` | The default svg3 document shown on load |
| `src/wasm/` | wasm-pack output — generated, git-ignored |

> The renderer itself lives in the [`svg3-web`](../svg3-web) Rust crate (wasm
> `cdylib`); this package is only the host shell + UI.

## Notes & limitations

- **WebGPU-only**, by design — wgpu's WebGL backend in a worker is unreliable.
- The 3D orbit camera applies only to documents that opt into the extension with
  `extension="pupiltong"` on the root; plain SVG renders flat.
- Like the rest of svg3, the **root `viewBox` is ignored** (1 user unit = 1
  device pixel), so a plain `<svg width height>` document paints at its pixel
  coordinates within the canvas.
- If a constrained CI environment can't download `wasm-opt`, set
  `[package.metadata.wasm-pack.profile.release] wasm-opt = false` in
  `svg3-web/Cargo.toml` (at the cost of a larger `.wasm`).
