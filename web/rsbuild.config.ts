import { defineConfig } from '@rsbuild/core';

// GitHub Pages serves this project site from `/<repo>/`. Set PAGES=1 for a
// production build so the bundle, the module worker chunk and the
// `svg3_web_bg.wasm` asset all resolve under `/svg3/`. Local `dev`/`preview`
// (PAGES unset) serve from the root.
const base = process.env.PAGES === '1' ? '/svg3/' : '/';

export default defineConfig({
  source: {
    entry: { index: './src/main.ts' },
  },
  html: {
    template: './index.html',
  },
  output: {
    // `target: 'web'` makes Rsbuild emit the worker as a real module worker.
    // Rspack's `experiments.asyncWebAssembly` is on by default, so the
    // wasm-pack ES module (which fetches its own `_bg.wasm` via
    // `new URL(..., import.meta.url)`) is bundled with no extra config.
    target: 'web',
    assetPrefix: base,
  },
  server: {
    base,
  },
});
