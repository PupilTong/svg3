// Message protocol between the UI thread (`main.ts`) and the render worker
// (`worker.ts`). All payloads are structured-clone friendly; the only
// transferable is the `OffscreenCanvas` in the `init` message.
//
// Sizes in `init` / `resize` are **physical** (device) pixels — the UI thread
// multiplies CSS pixels by `devicePixelRatio` before sending, so the worker can
// configure the surface directly without knowing about the DOM.

export type ToWorker =
  | { type: 'init'; canvas: OffscreenCanvas; width: number; height: number }
  | { type: 'source'; text: string }
  | { type: 'orbit'; dyaw: number; dpitch: number }
  | { type: 'zoom'; factor: number }
  | { type: 'reset' }
  | { type: 'resize'; width: number; height: number };

export type FromWorker =
  | { type: 'ready' }
  | { type: 'status'; message: string }
  | { type: 'error'; message: string; fatal: boolean }
  | { type: 'camera'; yaw: number; pitch: number; distance: number };
