// GPU precipitation: one InstancedMesh of quads, animated entirely in the
// vertex/fragment stage from a handful of uniforms. Each instance derives its
// spawn position, speed and phase from a per-instance PCG hash, falls under a
// time uniform, is sheared by horizontal wind, and wraps inside a box around
// the diorama via `mod`. CPU cost per frame: 4 uniform writes, no matrix work.
import * as THREE from 'three/webgpu';
import { float, hash, instanceIndex, mix, positionLocal, uniform, vec3, vec4 } from 'three/tsl';
import { precipLook, weatherLook } from '../designTokens';
import type { PrecipType } from './environment';

export type PrecipitationSystem = {
  update(type: PrecipType, intensity: number, windSpeedMs: number, windDirRad: number, dtSeconds: number): void;
  object3d: THREE.Object3D;
};

export type PrecipOpts = {
  boxX: number;
  boxY: number;
  boxZ: number;
  count: number;
  // Per-scene streak/flake overrides. The city framing needs heavier rain
  // fäden / bigger snow flocken than the room to read at the pulled-back scale.
  rainSx: number;
  rainSy: number;
  snowSx: number;
  snowSy: number;
};

const RAIN_SPEED = 9; // m/s fall
const SNOW_SPEED = 1.1;

export function createPrecipitation(opts?: Partial<PrecipOpts>): PrecipitationSystem {
  const COUNT = opts?.count ?? precipLook.room.count;
  const BOX = {
    x: opts?.boxX ?? precipLook.room.boxX,
    y: opts?.boxY ?? precipLook.room.boxY,
    z: opts?.boxZ ?? precipLook.room.boxZ,
  } as const;
  const timeU = uniform(0);
  const snowU = uniform(0); // 0 rain, 1 snow
  const windU = uniform(new THREE.Vector2(0, 0)); // horizontal drift m/s (scene x/z)
  const countU = uniform(0); // active fraction 0..1

  const geo = new THREE.PlaneGeometry(1, 1);
  const mat = new THREE.MeshBasicNodeMaterial({ transparent: true, depthWrite: false });
  mat.fog = false;
  mat.side = THREE.DoubleSide;
  {
    // TSL nodes aren't modellable with @types/three r185 — runtime-typed.
    type N = any;
    const f = (n: unknown): N => n as N;
    // hash() (three/tsl math/Hash.js) takes a node seed → float in [0,1). Salt
    // the instance index with distinct integers for decorrelated per-instance
    // draws (PCG decorrelates small offsets well).
    const rnd = (salt: number): N => f(hash(instanceIndex.add(salt)));

    const x0 = rnd(1).mul(BOX.x).sub(BOX.x / 2);
    const z0 = rnd(2).mul(BOX.z).sub(BOX.z / 2);
    const y0 = rnd(3).mul(BOX.y);

    const speed = mix(float(RAIN_SPEED), float(SNOW_SPEED), snowU).mul(rnd(4).mul(0.4).add(0.8));
    const fallen = f(timeU).mul(speed);
    // Fall from y0, wrap into [0, BOX.y).
    const y = f(y0.sub(fallen).mod(BOX.y));
    // Seconds airborne (approx) drives wind shear and snow wobble.
    const drift = f(fallen.div(speed));
    const wob = f(rnd(5).mul(6.28));
    const wobble = f(timeU.add(wob)).sin().mul(snowU).mul(0.4);

    // Wind shear on x/z; wrap centred on the box.
    const x = f(x0.add(f(windU.x).mul(drift)).add(wobble).add(BOX.x / 2).mod(BOX.x).sub(BOX.x / 2));
    const z = f(z0.add(f(windU.y).mul(drift)).add(BOX.z / 2).mod(BOX.z).sub(BOX.z / 2));

    // rain: thin vertical streak; snow: small square. Sizes are per-scene
    // (opts override the shared room defaults so the city can run heavier).
    const rainSx = opts?.rainSx ?? precipLook.rainSx;
    const rainSy = opts?.rainSy ?? precipLook.rainSy;
    const snowSx = opts?.snowSx ?? precipLook.snowSx;
    const snowSy = opts?.snowSy ?? precipLook.snowSy;
    const sx = mix(float(rainSx), float(snowSx), snowU);
    const sy = mix(float(rainSy), float(snowSy), snowU);
    const local = f(positionLocal).mul(vec3(sx, sy, float(1)));
    mat.positionNode = f(local).add(vec3(x, y, z));

    // Only a `countU` fraction of instances are active; hide the rest by alpha.
    const active = f(rnd(6).lessThan(countU));
    const col = mix(
      vec3(...hexToRgb01(weatherLook.rainColor)),
      vec3(...hexToRgb01(weatherLook.snowColor)),
      snowU,
    );
    const alpha = mix(float(precipLook.rainAlpha), float(precipLook.snowAlpha), snowU);
    mat.colorNode = vec4(col, f(alpha).mul(active.select(float(1), float(0))));
  }

  const mesh = new THREE.InstancedMesh(geo, mat, COUNT);
  mesh.frustumCulled = false;
  mesh.position.y = 0;

  let clock = 0;
  return {
    object3d: mesh,
    update(type: PrecipType, intensity: number, windSpeedMs: number, windDirRad: number, dt: number): void {
      clock += dt;
      timeU.value = clock;
      mesh.visible = type !== 'none' && intensity > 0.01;
      countU.value = type === 'none' ? 0 : 0.15 + 0.85 * intensity;
      snowU.value = type === 'snow' ? 1 : 0;
      // Wind blows *toward* windDirRad + PI; project onto scene x/z, damped.
      const toward = windDirRad + Math.PI;
      windU.value.set(Math.sin(toward) * windSpeedMs * 0.6, -Math.cos(toward) * windSpeedMs * 0.6);
    },
  };
}

function hexToRgb01(hex: number): [number, number, number] {
  return [((hex >> 16) & 0xff) / 255, ((hex >> 8) & 0xff) / 255, (hex & 0xff) / 255];
}
