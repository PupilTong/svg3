import './styles.css';
import { DEFAULT_SVG } from './demo';
import type { FromWorker, ToWorker } from './protocol';

const canvas = document.getElementById('canvas') as HTMLCanvasElement;
const sourceEl = document.getElementById('source') as HTMLTextAreaElement;
const statusEl = document.getElementById('status') as HTMLElement;
const cameraEl = document.getElementById('camera') as HTMLElement;
const resetBtn = document.getElementById('reset') as HTMLButtonElement;
const gpuBadge = document.getElementById('gpu-badge') as HTMLElement;
const overlay = document.getElementById('overlay') as HTMLElement;
const overlayTitle = document.getElementById('overlay-title') as HTMLElement;
const overlayBody = document.getElementById('overlay-body') as HTMLElement;

const DRAG_ORBIT_SPEED = 0.005; // radians per pixel
const KEY_ORBIT_STEP = 0.08; // radians per arrow press
const WHEEL_ZOOM_STEP = 0.88; // eye-distance multiplier per wheel "line"

sourceEl.value = DEFAULT_SVG;

function showOverlay(title: string, body: string) {
  overlayTitle.textContent = title;
  overlayBody.textContent = body;
  overlay.hidden = false;
}

// WebGPU is required: wgpu renders through `navigator.gpu` inside the worker.
// Detect up front and skip spawning the worker if it's missing.
if (!('gpu' in navigator)) {
  gpuBadge.textContent = 'WebGPU unavailable';
  gpuBadge.className = 'badge badge--bad';
  statusEl.textContent = 'WebGPU not available';
  sourceEl.disabled = true;
  showOverlay(
    'WebGPU required',
    'This demo renders with WebGPU, which this browser doesn’t expose. ' +
      'Try the latest Chrome or Edge (113+), Safari 18+, or Firefox with WebGPU enabled.',
  );
} else {
  gpuBadge.textContent = 'WebGPU';
  gpuBadge.className = 'badge badge--ok';
  boot();
}

function boot() {
  const worker = new Worker(new URL('./worker.ts', import.meta.url), {
    type: 'module',
  });

  // Hand the canvas to the worker — from here only the worker draws to it.
  const offscreen = canvas.transferControlToOffscreen();

  const send = (msg: ToWorker, transfer: Transferable[] = []) =>
    worker.postMessage(msg, transfer);

  // The canvas backing store is sized in physical (device) pixels so HiDPI
  // screens stay crisp; CSS keeps the element at its logical size.
  const physicalSize = () => {
    const dpr = Math.max(1, window.devicePixelRatio || 1);
    const rect = canvas.getBoundingClientRect();
    return {
      width: Math.max(1, Math.round(rect.width * dpr)),
      height: Math.max(1, Math.round(rect.height * dpr)),
    };
  };

  const initial = physicalSize();
  send({ type: 'init', canvas: offscreen, width: initial.width, height: initial.height }, [
    offscreen,
  ]);
  send({ type: 'source', text: DEFAULT_SVG });

  worker.onmessage = (ev: MessageEvent<FromWorker>) => {
    const msg = ev.data;
    switch (msg.type) {
      case 'ready':
        statusEl.textContent = 'ready';
        statusEl.classList.remove('status--error');
        break;
      case 'status':
        statusEl.textContent = msg.message;
        statusEl.classList.toggle('status--error', msg.message.startsWith('parse error'));
        break;
      case 'camera':
        cameraEl.textContent = `cam ${deg(msg.yaw)}°/${deg(msg.pitch)}° · d${Math.round(
          msg.distance,
        )}`;
        break;
      case 'error':
        statusEl.textContent = msg.message;
        statusEl.classList.add('status--error');
        if (msg.fatal) {
          showOverlay('Rendering stopped', `${msg.message}\n\nReload the page to try again.`);
        }
        break;
    }
  };

  // Editor → live render (debounced so each keystroke doesn't reparse).
  let debounce = 0;
  sourceEl.addEventListener('input', () => {
    clearTimeout(debounce);
    debounce = window.setTimeout(() => send({ type: 'source', text: sourceEl.value }), 180);
  });

  // Pointer drag → orbit.
  let dragging = false;
  let lastX = 0;
  let lastY = 0;
  canvas.addEventListener('pointerdown', (e) => {
    dragging = true;
    lastX = e.clientX;
    lastY = e.clientY;
    canvas.setPointerCapture(e.pointerId);
  });
  canvas.addEventListener('pointermove', (e) => {
    if (!dragging) return;
    send({
      type: 'orbit',
      dyaw: (e.clientX - lastX) * DRAG_ORBIT_SPEED,
      dpitch: (e.clientY - lastY) * DRAG_ORBIT_SPEED,
    });
    lastX = e.clientX;
    lastY = e.clientY;
  });
  const endDrag = (e: PointerEvent) => {
    dragging = false;
    try {
      canvas.releasePointerCapture(e.pointerId);
    } catch {
      /* pointer already released */
    }
  };
  canvas.addEventListener('pointerup', endDrag);
  canvas.addEventListener('pointercancel', endDrag);

  // Wheel → zoom. Scroll up zooms in (smaller eye distance), matching the
  // native app: factor < 1 pulls the camera closer.
  canvas.addEventListener(
    'wheel',
    (e) => {
      e.preventDefault();
      send({ type: 'zoom', factor: Math.pow(WHEEL_ZOOM_STEP, -e.deltaY / 100) });
    },
    { passive: false },
  );

  // Arrow keys orbit, R resets — but never while typing in the editor.
  window.addEventListener('keydown', (e) => {
    if (document.activeElement === sourceEl) return;
    switch (e.key) {
      case 'ArrowLeft':
        send({ type: 'orbit', dyaw: -KEY_ORBIT_STEP, dpitch: 0 });
        e.preventDefault();
        break;
      case 'ArrowRight':
        send({ type: 'orbit', dyaw: KEY_ORBIT_STEP, dpitch: 0 });
        e.preventDefault();
        break;
      case 'ArrowUp':
        send({ type: 'orbit', dyaw: 0, dpitch: -KEY_ORBIT_STEP });
        e.preventDefault();
        break;
      case 'ArrowDown':
        send({ type: 'orbit', dyaw: 0, dpitch: KEY_ORBIT_STEP });
        e.preventDefault();
        break;
      case 'r':
      case 'R':
        send({ type: 'reset' });
        break;
    }
  });

  resetBtn.addEventListener('click', () => send({ type: 'reset' }));

  // Keep the backing store matched to the element's physical size (debounced —
  // ResizeObserver can fire rapidly during a drag-resize).
  let resizeTimer = 0;
  const observer = new ResizeObserver(() => {
    clearTimeout(resizeTimer);
    resizeTimer = window.setTimeout(() => {
      const s = physicalSize();
      send({ type: 'resize', width: s.width, height: s.height });
    }, 80);
  });
  observer.observe(canvas);
}

function deg(rad: number): string {
  return ((rad * 180) / Math.PI).toFixed(0);
}
