// Render worker: owns the OffscreenCanvas and the wasm `WebRenderer`. The GPU
// render (wgpu → offscreen texture → read-back) and the canvas draw
// (`putImageData` via a 2D context) both happen here, off the main thread. The
// UI thread only forwards input messages.
import init, { WebRenderer } from './wasm/svg3_web.js';
import type { FromWorker, ToWorker } from './protocol';

// `self` in a dedicated worker is a `DedicatedWorkerGlobalScope`. We only need
// `postMessage`/`onmessage`, so we narrow it here rather than pull in the
// WebWorker lib (which clashes with the DOM lib in a single tsconfig).
interface WorkerCtx {
  postMessage(message: FromWorker): void;
  onmessage: ((ev: MessageEvent) => void) | null;
}
const ctx = self as unknown as WorkerCtx;
const post = (msg: FromWorker) => ctx.postMessage(msg);

let renderer: WebRenderer | null = null;
let initializing = false;

let canvas: OffscreenCanvas | null = null;
let ctx2d: OffscreenCanvasRenderingContext2D | null = null;
let width = 1;
let height = 1;

// All non-init messages queue here and are drained by `pump`. `render_frame`
// borrows the wasm `WebRenderer` across its async read-back, so a `&mut self`
// call (orbit/zoom/resize/set_source) made *during* that await would trip
// wasm-bindgen's "recursive use" aliasing guard. The pump enforces the
// invariant: apply every queued mutation synchronously, then await exactly one
// render — messages arriving during that await only enqueue.
const inbox: ToWorker[] = [];
let pumping = false;

async function bootstrap(c: OffscreenCanvas, w: number, h: number) {
  initializing = true;
  canvas = c;
  width = Math.max(1, w);
  height = Math.max(1, h);
  canvas.width = width;
  canvas.height = height;
  ctx2d = canvas.getContext('2d');
  try {
    // Instantiate the wasm module (runs `start()`: panic hook + console log).
    await init();
    renderer = await WebRenderer.create(width, height);
    post({ type: 'ready' });
    void pump(); // drain anything that queued during init (e.g. the first source)
  } catch (e) {
    post({ type: 'error', message: errorText(e), fatal: true });
  }
}

function postCamera() {
  if (!renderer) return;
  post({
    type: 'camera',
    yaw: renderer.camera_yaw(),
    pitch: renderer.camera_pitch(),
    distance: renderer.camera_distance(),
  });
}

/** Apply one state-changing message to the wasm renderer (synchronous, `&mut`).
 *  Returns whether the change requires a redraw. */
function applyState(msg: ToWorker): boolean {
  if (!renderer) return false;
  switch (msg.type) {
    case 'source': {
      const status = renderer.set_source(msg.text);
      post({ type: 'status', message: status });
      postCamera();
      return true;
    }
    case 'orbit':
      renderer.orbit(msg.dyaw, msg.dpitch);
      postCamera();
      return true;
    case 'zoom':
      renderer.zoom(msg.factor);
      postCamera();
      return true;
    case 'reset':
      renderer.reset();
      postCamera();
      return true;
    case 'resize':
      width = Math.max(1, msg.width);
      height = Math.max(1, msg.height);
      if (canvas) {
        canvas.width = width;
        canvas.height = height;
      }
      renderer.resize(width, height);
      return true;
    case 'init':
      return false;
  }
  return false;
}

async function pump() {
  if (pumping || !renderer || !ctx2d) return;
  pumping = true;
  try {
    while (inbox.length) {
      // Drain every queued mutation first — no await between them and the
      // render, so no `&mut` wasm call overlaps the in-flight `render_frame`.
      let dirty = false;
      while (inbox.length) {
        const msg = inbox.shift();
        if (msg) dirty = applyState(msg) || dirty;
      }
      if (dirty) {
        const w = width;
        const h = height;
        const bytes = await renderer.render_frame();
        const data = new Uint8ClampedArray(bytes.buffer, bytes.byteOffset, bytes.byteLength);
        if (data.length === w * h * 4) {
          ctx2d.putImageData(new ImageData(data, w, h), 0, 0);
        }
      }
    }
  } catch (e) {
    // A wasm panic surfaces as a JS exception; treat it as fatal.
    post({ type: 'error', message: errorText(e), fatal: true });
  } finally {
    pumping = false;
  }
}

ctx.onmessage = (ev: MessageEvent) => {
  const msg = ev.data as ToWorker;
  if (msg.type === 'init') {
    if (!initializing && !renderer) void bootstrap(msg.canvas, msg.width, msg.height);
    return;
  }
  inbox.push(msg);
  void pump();
};

function errorText(e: unknown): string {
  if (typeof e === 'string') return e;
  if (e instanceof Error) return e.message;
  return String(e);
}
