// Applies an EnvironmentState to the KSW city scene. Uniform/mutable writes
// only — no geometry rebuilds, no allocations in the hot path.
//
// City-specific vs the room's applyEnvironment:
//  - fog goes through fogBase (animate() zoom-scales near/far each frame),
//  - the key-light direction is written IN-PLACE into t.currentSunDir, which
//    the shadow-follow rig (updateShadowFrustum) reads to place sun.position —
//    apply never touches sun.position itself,
//  - clouds are the two fbm domes sharing one sunDirUniform / coverageU / driftUV,
//  - night glow drives lampGlowU + the pooled forecourt/emergency point lights,
//  - there are no east-window light shafts (that is a room feature).

import * as THREE from 'three/webgpu';
import { cloudLook, kswScene, moonLight, nightGlow, nightSkyLook } from '../designTokens';
import { moonPhaseLightDir, type EnvironmentState } from '../environment/environment';
import { POLE_AXIS } from '../environment/applyEnvironment';
import type { PrecipitationSystem } from '../environment/precipitation';
import { windAmpU, windDirU, windAmplitude } from './windUniform';
import { impostorLightU } from './geo/treeImpostors';

export type CityEnvironmentTargets = {
  renderer: THREE.WebGPURenderer;
  fog: THREE.Fog;
  // apply writes BASE near/far here; animate() multiplies by the zoom factor.
  fogBase: { near: number; far: number };
  sun: THREE.DirectionalLight;
  // IN-PLACE: the shadow-follow rig reads this to place sun.position.
  currentSunDir: THREE.Vector3;
  hemi: THREE.HemisphereLight;
  skyMesh: {
    turbidity: { value: number };
    rayleigh: { value: number };
    mieCoefficient: { value: number };
    mieDirectionalG: { value: number };
    sunPosition: { value: THREE.Vector3 };
  };
  cloud: {
    sunDirUniform: { value: THREE.Vector3 };
    lit: { value: THREE.Color };
    shadow: { value: THREE.Color };
    coverageU: { value: number };
    driftUV: { value: THREE.Vector2 };
  };
  post: { saturationU: { value: number }; contrastU: { value: number }; godraysMixU: { value: number } };
  // apply writes baseOpacity + color; animate() applies the fade/cloudMix.
  mist: { mat: THREE.MeshBasicMaterial; cityMat: THREE.MeshBasicMaterial; baseOpacity: { value: number } };
  sunDisc: THREE.Mesh;
  moon: { mesh: THREE.Mesh; phaseDir: { value: THREE.Vector3 } };
  stars: { object3d: THREE.Object3D; material: { opacity: number } };
  lampLights: THREE.PointLight[];
  lampBaseIntensities: number[];
  precipitation: PrecipitationSystem;
  // scene.environmentIntensity base; animate() computes the final intensity.
  giBase: { value: number };
  exposure: (v: number) => void;
  scratch: { v3: THREE.Vector3; c1: THREE.Color; c2: THREE.Color };
  lampGlow: { value: number };
};

export function applyCityEnvironment(t: CityEnvironmentTargets, env: EnvironmentState, dtSeconds: number): void {
  const isDay = env.sunIntensity > 0.02;
  // scratch.v3 is reused (sunDir → moonDir); each consumer copies out before
  // the next set overwrites it.
  const sunDir = t.scratch.v3.set(env.sunDir[0], env.sunDir[1], env.sunDir[2]);

  // Sky
  t.skyMesh.turbidity.value = env.turbidity;
  t.skyMesh.rayleigh.value = env.rayleigh;
  t.skyMesh.mieCoefficient.value = env.mieCoefficient;
  t.skyMesh.mieDirectionalG.value = env.mieG;
  t.skyMesh.sunPosition.value.copy(sunDir);

  // Fog: write BASE values; animate() scales near/far with the zoom radius.
  t.fog.color.set(env.fogColor);
  t.fogBase.near = env.fogNear * kswScene.fogScale;
  t.fogBase.far = env.fogFar * kswScene.fogScale;
  t.exposure(env.exposure);

  // Key light: sun by day, moon by night. Direction goes IN-PLACE into
  // currentSunDir — the shadow-follow rig positions the light from it.
  if (isDay) {
    t.currentSunDir.copy(sunDir);
    t.sun.color.set(env.sunColor);
    t.sun.intensity = Math.max(env.sunIntensity, 0.05);
    t.cloud.sunDirUniform.value.copy(sunDir);
    t.cloud.lit.value.set(env.sunColor).lerp(t.scratch.c1.set(cloudLook.litWhite), cloudLook.litWhiteMix);
    t.cloud.shadow.value.set(cloudLook.shadowBase).lerp(t.scratch.c2.set(env.sunColor), 0.15);
  } else {
    const moonDir = t.scratch.v3.set(env.moonDir[0], Math.max(env.moonDir[1], 0.15), env.moonDir[2]).normalize();
    t.currentSunDir.copy(moonDir);
    t.sun.color.set(moonLight.color);
    t.sun.intensity = Math.max(env.moonIntensity, 0.12);
    t.cloud.sunDirUniform.value.copy(moonDir);
    t.cloud.lit.value.set(cloudLook.nightLit);
    t.cloud.shadow.value.set(cloudLook.nightShadow);
  }

  // Hemisphere (hemiCut already folded into the keyframe hemiIntensity values)
  t.hemi.color.set(env.hemiSky);
  t.hemi.groundColor.set(env.hemiGround);
  t.hemi.intensity = env.hemiIntensity;

  // Impostor light calibration: the far-LOD tree atlas is baked unlit, so
  // without this it reads paler/flatter than the fully-shaded near trees at
  // the 150 m LOD handoff (see treeImpostors.ts). Follow the same sun/hemi
  // state that just fed the real lights, weighted so midday-clear settles
  // near white (matching the unlit bake, i.e. today's look is unchanged at
  // the reference condition). Retuned in the Task 6 screenshot pass
  // (2026-07-06): sun 0.7 + sky 0.45 — at the previous 0.6/0.4 the far field
  // read a touch darker than the sun-lit full trees at the 150 m handoff
  // (scratch/tree-polish/handoff-fix8.png vs -fix9.png).
  // Night retune (2026-07-06): the fixed 0.45 hemi weight ignored the actual
  // hemi intensity (0.28 day vs 0.166 night) and the moon key fed the same
  // 0.7 weight as the sun — impostor trees glowed mint-green over the dark
  // city. Scale the hemi term by the live intensity (1.6·0.28 ≈ the tuned
  // 0.45 at the day reference, so the day handoff is unchanged), damp the
  // key weight at night, and clamp so a >1 midday sum can no longer blow the
  // pre-lit atlas to white.
  // Elevation-aware sun weight: the unlit atlas can't self-shadow, so at low
  // sun (golden hour) full sun colour painted EVERY tree a glowing amber blob
  // over the dark city — real crowns are mostly shadowed then. Fade the sun
  // term in with elevation (full above ~20°).
  const sunElevW = isDay ? Math.min(1, Math.max(0, env.sunDir[1] / 0.35)) : 1;
  const sunW = 0.7 * Math.min(1, t.sun.intensity) * (isDay ? sunElevW : 0.3);
  // Night factor 0.22: the night hemi runs hot (1.3) to keep the CITY readable
  // under AgX; feeding that raw into the unlit impostor atlas made every far
  // tree glow teal brighter than the lamps. The hemi term ALSO ramps with sun
  // elevation by day — at dawn/dusk the physically-lit scene sits in the AgX
  // toe while a flat 0.48 hemi painted the far trees as pastel confetti.
  const hemiW = 1.6 * t.hemi.intensity * (isDay ? 0.35 + 0.65 * sunElevW : 0.22);
  impostorLightU.value.setRGB(
    Math.min(1, t.sun.color.r * sunW + t.hemi.color.r * hemiW),
    Math.min(1, t.sun.color.g * sunW + t.hemi.color.g * hemiW),
    Math.min(1, t.sun.color.b * sunW + t.hemi.color.b * hemiW),
  );

  // Clouds
  t.cloud.coverageU.value = env.cloudCoverage;
  t.cloud.driftUV.value.x += env.cloudDriftDir[0] * env.cloudDriftSpeed * dtSeconds;
  t.cloud.driftUV.value.y += env.cloudDriftDir[1] * env.cloudDriftSpeed * dtSeconds;

  // Post
  t.post.saturationU.value = env.saturation;
  t.post.contrastU.value = env.contrast;
  t.post.godraysMixU.value = env.godraysMix;

  // Mist: apply writes color + base opacity; animate() applies fade / cloudMix.
  t.mist.mat.color.set(env.mistColor);
  t.mist.cityMat.color.set(env.mistColor);
  t.mist.baseOpacity.value = env.mistOpacity;

  // GI probe intensity base; animate() finalizes scene.environmentIntensity.
  t.giBase.value = env.giScale;

  // Sun disc: read env.* directly (scratch was reused above). Distance is the
  // city sky dome (nightSkyLook.city.sunDistance) — same dome as the moon; the
  // old kswScene.domeRadius (hero 400) parked it low over the city as a white ball.
  t.sunDisc.position.set(env.sunDir[0], env.sunDir[1], env.sunDir[2]).multiplyScalar(nightSkyLook.city.sunDistance);
  t.sunDisc.visible = env.sunDir[1] > 0.015;

  // Moon
  t.moon.mesh.position
    .set(env.moonDir[0], env.moonDir[1], env.moonDir[2])
    .multiplyScalar(nightSkyLook.city.moonDistance);
  t.moon.mesh.visible = env.moonDir[1] > 0.02 && env.starVisibility > 0.02;
  t.moon.phaseDir.value.set(...moonPhaseLightDir(env.moonPhase));

  // Stars: real dome rotation around the celestial pole.
  t.stars.material.opacity = 0.85 * env.starVisibility;
  t.stars.object3d.visible = env.starVisibility > 0.01;
  t.stars.object3d.rotation.set(0, 0, 0);
  t.stars.object3d.rotateOnWorldAxis(POLE_AXIS, env.siderealAngleRad);

  // Night glow: shared uniform (batched windows/bulbs) + the pooled point lights.
  t.lampGlow.value = env.lampOn01;
  for (let i = 0; i < t.lampLights.length; i++) {
    t.lampLights[i].intensity = t.lampBaseIntensities[i] * nightGlow.boost * env.lampOn01;
  }

  // Precipitation
  t.precipitation.update(env.precipType, env.precipIntensity, env.windSpeedMs, env.windDirRad, dtSeconds);

  // Wind uniforms: direction convention matches precipitation.ts (wind blows TOWARD,
  // so we add PI to windDirRad and use sin/cos with negative cosine for x/z).
  windAmpU.value = windAmplitude(env.windSpeedMs);
  const toward = env.windDirRad + Math.PI;
  (windDirU.value as THREE.Vector2).set(Math.sin(toward), -Math.cos(toward));
}
