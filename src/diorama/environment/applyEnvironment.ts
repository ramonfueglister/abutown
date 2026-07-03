// Applies an EnvironmentState to the live scene. Uniform/mutable writes only —
// no geometry rebuilds, no allocations in the hot path.

import * as THREE from 'three/webgpu';
import { moonLight, nightGlow } from '../designTokens';
import { moonPhaseLightDir, type EnvironmentState } from './environment';
import type { PrecipitationSystem } from './precipitation';

export type EnvironmentTargets = {
  renderer: THREE.WebGPURenderer;
  fog: THREE.Fog;
  sun: THREE.DirectionalLight;
  hemi: THREE.HemisphereLight;
  skyMesh: {
    turbidity: { value: number };
    rayleigh: { value: number };
    mieCoefficient: { value: number };
    mieDirectionalG: { value: number };
    sunPosition: { value: THREE.Vector3 };
  };
  cloudUniforms: {
    lightDir: { value: THREE.Vector3 };
    lit: { value: THREE.Color };
    shadow: { value: THREE.Color };
    coverage: { value: number };
    driftUV: { value: THREE.Vector2 };
  };
  postUniforms: { saturation: { value: number }; contrast: { value: number }; godraysMix: { value: number } };
  mistMaterial: THREE.MeshBasicMaterial;
  sunDisc: THREE.Mesh;
  moonDisc: THREE.Mesh;
  moonPhaseDir: { value: THREE.Vector3 };
  lampLight: THREE.PointLight;
  lampBulb: THREE.Mesh;
  stars: THREE.Points;
  starsMaterial: THREE.PointsMaterial;
  shaftMaterial: THREE.MeshBasicMaterial;
  shafts: THREE.Mesh[];
  shaftWindows: THREE.Vector3[];
  precipitation: PrecipitationSystem;
  scratch: { v3: THREE.Vector3; c1: THREE.Color; c2: THREE.Color };
};

const CLOUD_SHADOW_BASE = 0x6e8092;
const CLOUD_NIGHT_LIT = 0x9fb2cc;
const CLOUD_NIGHT_SHADOW = 0x39485c;

export function applyEnvironment(t: EnvironmentTargets, env: EnvironmentState, dtSeconds: number): void {
  // scratch.v3 is reused three times below (sunDir → moonDir → per-window shaft
  // "down"). Each consumer copies the value out (uniform .copy / light .position)
  // before the next set overwrites it, so the aliasing is safe.
  const sunDir = t.scratch.v3.set(env.sunDir[0], env.sunDir[1], env.sunDir[2]);
  const isDay = env.sunIntensity > 0.02;

  // Sky + fog + exposure
  t.skyMesh.turbidity.value = env.turbidity;
  t.skyMesh.rayleigh.value = env.rayleigh;
  t.skyMesh.mieCoefficient.value = env.mieCoefficient;
  t.skyMesh.mieDirectionalG.value = env.mieG;
  t.skyMesh.sunPosition.value.copy(sunDir);
  t.fog.color.set(env.fogColor);
  t.fog.near = env.fogNear;
  t.fog.far = env.fogFar;
  t.renderer.toneMappingExposure = env.exposure;

  // Key light: sun by day, moon by night (one shadow-casting light, like the prototype)
  if (isDay) {
    t.sun.position.copy(sunDir).multiplyScalar(12);
    t.sun.color.set(env.sunColor);
    t.sun.intensity = Math.max(env.sunIntensity, 0.05);
    t.cloudUniforms.lightDir.value.copy(sunDir);
    t.cloudUniforms.lit.value.set(env.sunColor).lerp(t.scratch.c1.set(0xffffff), 0.3);
    t.cloudUniforms.shadow.value.set(CLOUD_SHADOW_BASE).lerp(t.scratch.c2.set(env.sunColor), 0.15);
  } else {
    const moonDir = t.scratch.v3.set(env.moonDir[0], Math.max(env.moonDir[1], 0.15), env.moonDir[2]).normalize();
    t.sun.position.copy(moonDir).multiplyScalar(12);
    t.sun.color.set(moonLight.color);
    t.sun.intensity = Math.max(env.moonIntensity, 0.12);
    t.cloudUniforms.lightDir.value.copy(moonDir);
    t.cloudUniforms.lit.value.set(CLOUD_NIGHT_LIT);
    t.cloudUniforms.shadow.value.set(CLOUD_NIGHT_SHADOW);
  }

  // Hemisphere + GI scale
  t.hemi.color.set(env.hemiSky);
  t.hemi.groundColor.set(env.hemiGround);
  t.hemi.intensity = env.hemiIntensity;

  // Clouds
  t.cloudUniforms.coverage.value = env.cloudCoverage;
  t.cloudUniforms.driftUV.value.x += env.cloudDriftDir[0] * env.cloudDriftSpeed * dtSeconds;
  t.cloudUniforms.driftUV.value.y += env.cloudDriftDir[1] * env.cloudDriftSpeed * dtSeconds;

  // Post
  t.postUniforms.saturation.value = env.saturation;
  t.postUniforms.contrast.value = env.contrast;
  t.postUniforms.godraysMix.value = env.godraysMix;

  // Mist
  t.mistMaterial.color.set(env.mistColor);
  t.mistMaterial.opacity = env.mistOpacity;

  // Discs (read env.* directly, so the scratch reuse above doesn't affect them)
  t.sunDisc.position.set(env.sunDir[0], env.sunDir[1], env.sunDir[2]).multiplyScalar(60);
  t.sunDisc.visible = env.sunDir[1] > 0.015;
  t.moonDisc.position.set(env.moonDir[0], env.moonDir[1], env.moonDir[2]).multiplyScalar(60);
  t.moonDisc.visible = env.moonDir[1] > 0.02 && env.starVisibility > 0.02;
  t.moonPhaseDir.value.set(...moonPhaseLightDir(env.moonPhase));

  // Lamp (warm windows)
  t.lampLight.intensity = nightGlow.lampIntensity * 1.2 * env.lampOn01;
  t.lampBulb.visible = env.lampOn01 > 0.05;

  // Stars: real dome rotation around the celestial pole
  t.starsMaterial.opacity = 0.85 * env.starVisibility;
  t.stars.visible = env.starVisibility > 0.01;
  t.stars.rotation.set(0, 0, 0);
  t.stars.rotateOnWorldAxis(POLE_AXIS, env.siderealAngleRad);

  // East-window shafts: re-aim along the live sun direction, fade by shaft01
  t.shaftMaterial.opacity = 0.07 * env.shaft01;
  for (let i = 0; i < t.shafts.length; i++) {
    const win = t.shaftWindows[i];
    const down = t.scratch.v3.set(-env.sunDir[0], -env.sunDir[1], -env.sunDir[2]);
    if (down.y > -0.05) {
      t.shafts[i].visible = false;
      continue;
    }
    t.shafts[i].visible = env.shaft01 > 0.01;
    const k = (win.y - 0.16) / -down.y;
    const poolV = scratchPool.copy(win).addScaledVector(down, k);
    t.shafts[i].position.copy(win).add(poolV).multiplyScalar(0.5);
    t.shafts[i].lookAt(poolV);
    t.shafts[i].scale.z = win.distanceTo(poolV) / SHAFT_BASE_LEN;
  }

  // Precipitation
  t.precipitation.update(env.precipType, env.precipIntensity, env.windSpeedMs, env.windDirRad, dtSeconds);
}

// Celestial pole for latitude 47.5°: in scene coords the pole sits toward
// north (-z) at elevation = latitude.
const LAT_RAD = (47.499 * Math.PI) / 180;
const POLE_AXIS = new THREE.Vector3(0, Math.sin(LAT_RAD), -Math.cos(LAT_RAD)).normalize();
const SHAFT_BASE_LEN = 3; // shafts are built with unit length 3, scaled per frame
const scratchPool = new THREE.Vector3();
