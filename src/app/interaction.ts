import {
  panCameraTarget,
  zoomCameraAt,
  type CameraState,
} from '../cameraController';

export type Coord = { x: number; y: number };

export type AttachMapInteractionOptions = {
  canvas: HTMLCanvasElement;
  camera: CameraState;
  constrainCamera: (allowOverscroll: boolean) => void;
  selectAtScreenPoint: (point: Coord) => void;
  minScale: number | (() => number);
  maxScale: number;
};

export function attachMapInteraction(options: AttachMapInteractionOptions): void {
  let pointerDown: Coord | null = null;
  const { canvas, camera } = options;

  canvas.addEventListener('pointerdown', (event) => {
    camera.dragging = true;
    pointerDown = { x: event.clientX, y: event.clientY };
    camera.lastX = event.clientX;
    camera.lastY = event.clientY;
    canvas.setPointerCapture(event.pointerId);
  });

  canvas.addEventListener('pointermove', (event) => {
    if (!camera.dragging) return;
    panCameraTarget(camera, event.clientX - camera.lastX, event.clientY - camera.lastY);
    options.constrainCamera(true);
    camera.lastX = event.clientX;
    camera.lastY = event.clientY;
  });

  canvas.addEventListener('pointerup', (event) => {
    const clickDistance = pointerDown ? Math.hypot(event.clientX - pointerDown.x, event.clientY - pointerDown.y) : Infinity;
    camera.dragging = false;
    if (clickDistance < 4) options.selectAtScreenPoint({ x: event.clientX, y: event.clientY });
    pointerDown = null;
    options.constrainCamera(false);
  });

  canvas.addEventListener('pointercancel', () => {
    camera.dragging = false;
    pointerDown = null;
    options.constrainCamera(false);
  });

  canvas.addEventListener('wheel', (event) => {
    event.preventDefault();
    const minScale = typeof options.minScale === 'function' ? options.minScale() : options.minScale;
    zoomCameraAt(camera, { x: event.clientX, y: event.clientY }, event.deltaY, event.deltaMode, {
      minScale,
      maxScale: options.maxScale,
    });
    options.constrainCamera(false);
  }, { passive: false });
}
