// Pure per-vehicle wheel state: roll angle from dead-reckoned speed and a
// low-pass-filtered front steer angle from the yaw rate. carLayer keeps one
// SpinState per vehicle id and mutates it in place each frame (no allocation).

import { SIM_DT } from './deadReckon';

export interface SpinState { theta: number; steer: number; lastTick: number; lastYaw: number }

export const MAX_STEER = 0.45;
/** Steer gain: full-lock (MAX_STEER) at a yaw rate of ~1 rad/s. */
const STEER_GAIN = 0.45;
/** Low-pass rate (1/s): how fast steer chases its target. */
const STEER_LP = 8;

export function initSpin(nowTick: number, yaw: number): SpinState {
  return { theta: 0, steer: 0, lastTick: nowTick, lastYaw: yaw };
}

function wrapAngle(a: number): number {
  while (a > Math.PI) a -= 2 * Math.PI;
  while (a < -Math.PI) a += 2 * Math.PI;
  return a;
}

export function advanceSpin(st: SpinState, v: number, yaw: number, nowTick: number, wheelRadius: number): SpinState {
  const dt = (nowTick - st.lastTick) * SIM_DT;
  if (dt > 0) {
    st.theta += (v * dt) / wheelRadius;
    const yawRate = wrapAngle(yaw - st.lastYaw) / dt;
    const target = Math.max(-MAX_STEER, Math.min(MAX_STEER, yawRate * STEER_GAIN));
    st.steer += (target - st.steer) * Math.min(1, dt * STEER_LP);
    st.lastTick = nowTick;
    st.lastYaw = yaw;
  }
  return st;
}
