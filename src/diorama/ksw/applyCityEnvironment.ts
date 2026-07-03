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

  // Sun disc: read env.* directly (scratch was reused above).
  t.sunDisc.position.set(env.sunDir[0], env.sunDir[1], env.sunDir[2]).multiplyScalar(kswScene.domeRadius * 0.82);
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
}
