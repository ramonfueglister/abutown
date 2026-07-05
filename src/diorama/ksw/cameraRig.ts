// Pure orbit-camera math for the KSW diorama: mouse-wheel dolly zoom,
// left-drag rotation (free yaw, softly clamped pitch), and the roof-fade
// signal derived from the zoom radius. No three.js dependency — fully
// unit-testable.

export type CameraRigState = {
  yaw: number;
  pitch: number;
  radius: number;
  target: [number, number, number];
};

export type RigConfig = {
  radiusMin: number;
  radiusMax: number;
  zoomSpeed: number;
  dragSpeed: number;
  pitchMin: number;
  pitchMax: number;
  roofFadeNear: number;
  roofFadeFar: number;
  // Age-of-Empires-style edge scrolling
  panMarginPx: number; // active zone at each viewport edge
  panSpeed: number; // world units/s at full ramp
  panBoundsX: number; // |target.x| clamp
  panBoundsZ: number; // |target.z| clamp
  keyRotateSpeed: number; // rad/s for Q/E keyboard rotation
};

const clamp = (v: number, lo: number, hi: number): number => Math.min(Math.max(v, lo), hi);

export function rigFromLookAt(
  position: [number, number, number],
  target: [number, number, number],
): CameraRigState {
  const dx = position[0] - target[0];
  const dy = position[1] - target[1];
  const dz = position[2] - target[2];
  const radius = Math.hypot(dx, dy, dz);
  return {
    yaw: Math.atan2(dx, dz),
    pitch: Math.asin(dy / radius),
    radius,
    target: [target[0], target[1], target[2]],
  };
}

export function rigPosition(s: CameraRigState): [number, number, number] {
  const horiz = s.radius * Math.cos(s.pitch);
  return [
    s.target[0] + horiz * Math.sin(s.yaw),
    s.target[1] + s.radius * Math.sin(s.pitch),
    s.target[2] + horiz * Math.cos(s.yaw),
  ];
}

export function applyZoom(s: CameraRigState, wheelDeltaY: number, cfg: RigConfig): CameraRigState {
  const radius = clamp(s.radius * Math.exp(wheelDeltaY * cfg.zoomSpeed), cfg.radiusMin, cfg.radiusMax);
  return { ...s, radius };
}

export function applyDrag(s: CameraRigState, dxPx: number, dyPx: number, cfg: RigConfig): CameraRigState {
  return {
    ...s,
    yaw: s.yaw - dxPx * cfg.dragSpeed,
    pitch: clamp(s.pitch + dyPx * cfg.dragSpeed, cfg.pitchMin, cfg.pitchMax),
  };
}

// 1 = roofs fully present (zoomed out), 0 = roofs gone (zoomed in).
export function roofFade(radius: number, cfg: RigConfig): number {
  const t = clamp((radius - cfg.roofFadeNear) / (cfg.roofFadeFar - cfg.roofFadeNear), 0, 1);
  return t * t * (3 - 2 * t);
}

// AoE2-style edge scrolling: cursor inside the edge margin yields a world-
// space pan velocity aligned with the screen (screen-right/up mapped through
// the camera yaw). Ramps linearly from 0 at the margin to panSpeed at the
// very edge; opposing edges cancel via the ramp being zero elsewhere.
export function edgePanVelocity(
  mouseX: number,
  mouseY: number,
  width: number,
  height: number,
  yaw: number,
  cfg: RigConfig,
): [number, number] {
  const m = cfg.panMarginPx;
  const ramp = (distToEdge: number): number => clamp(1 - distToEdge / m, 0, 1);
  const sx = ramp(width - 1 - mouseX) - ramp(mouseX); // +1 right edge, -1 left edge
  const sy = ramp(mouseY) - ramp(height - 1 - mouseY); // +1 top edge, -1 bottom edge
  if (sx === 0 && sy === 0) return [0, 0];
  // screen-right and screen-up (ground-projected) for an orbit camera at yaw
  const rightX = Math.cos(yaw);
  const rightZ = -Math.sin(yaw);
  const fwdX = -Math.sin(yaw);
  const fwdZ = -Math.cos(yaw);
  return [(sx * rightX + sy * fwdX) * cfg.panSpeed, (sx * rightZ + sy * fwdZ) * cfg.panSpeed];
}

export type PanKeys = { up: boolean; down: boolean; left: boolean; right: boolean };

// WASD/arrow keyboard pan: mirrors edgePanVelocity's screen->world basis so
// keyboard and edge scrolling agree exactly. Held direction flags collapse to
// screen axes (sx = right-left, sy = up-down), then project through the yaw.
export function keyboardPanVelocity(held: PanKeys, yaw: number, cfg: RigConfig): [number, number] {
  const sx = (held.right ? 1 : 0) - (held.left ? 1 : 0);
  const sy = (held.up ? 1 : 0) - (held.down ? 1 : 0);
  if (sx === 0 && sy === 0) return [0, 0];
  const rightX = Math.cos(yaw);
  const rightZ = -Math.sin(yaw);
  const fwdX = -Math.sin(yaw);
  const fwdZ = -Math.cos(yaw);
  return [(sx * rightX + sy * fwdX) * cfg.panSpeed, (sx * rightZ + sy * fwdZ) * cfg.panSpeed];
}

export function applyPan(
  s: CameraRigState,
  vx: number,
  vz: number,
  dt: number,
  cfg: RigConfig,
): CameraRigState {
  return {
    ...s,
    target: [
      clamp(s.target[0] + vx * dt, -cfg.panBoundsX, cfg.panBoundsX),
      s.target[1],
      clamp(s.target[2] + vz * dt, -cfg.panBoundsZ, cfg.panBoundsZ),
    ],
  };
}
