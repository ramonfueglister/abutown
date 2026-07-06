# Cities-Skylines-Style Cars — Design

Date: 2026-07-06 · Status: approved by user · Branch: `cs-style-cars`

## Goal

Replace the clay box cars in the traffic diorama with cars that look like
Cities: Skylines vehicles (reference screenshots: `research/Download.jpeg`,
`research/Download (1).jpeg`), including **visibly rotating wheels** and light
front-wheel steering. The user explicitly approved the **full CS look** — the
cars may depart from the diorama's matte clay language.

Visual traits extracted from the reference screenshots:

- Rounded low-poly bodies: sloped hood, raked windshield, curved roofline,
  trunk step — smooth-shaded, not boxes.
- Bright, highly reflective sky-blue glass (NOT dark glass).
- Real round wheels with dark tires and light hubcaps, visible under the body.
- Front/rear detail zones: dark grille, light headlights, red taillights,
  white license plates.
- Saturated glossy paint colours (yellow, maroon, blue, brown, green, beige,
  black, white/silver).
- Additional vehicle classes: vans / Sprinter-type minibuses.

## Scope

- `src/diorama/traffic/carModels.ts` — new geometry builders + variant table.
- `src/diorama/traffic/carLayer.ts` — body/glass/wheel instancing + wheel spin.
- `src/diorama/traffic/flowLayer.ts` — unchanged API (`buildCarGeometry()`
  keeps returning one representative merged geometry for the far-LOD impostor).
- `tests/traffic/carModels.test.ts` — updated + extended.

Out of scope: buses/trams/trucks driven by other layers, pedestrians, road
rendering, any server change. No network or protocol change — purely a
client-side render upgrade.

## Architecture

### Geometry (procedural, no external assets)

Each variant body is built by **extruding a 2-D side profile** (polyline in
the y/z plane: front bumper → hood → windshield → roof → rear glass → trunk →
rear bumper) across the car width, with the top face insets ("shoulder"
bevel) so silhouettes read rounded, then `computeVertexNormals()` with
smooth-shaded body panels. This stays 100 % procedural like the rest of the
diorama.

Per variant, **two** geometries:

1. **Body** — vertex-colour zones baked in:
   - body panels **white** (tinted per instance via `setColorAt`, exactly the
     existing mechanism),
   - grille **dark**, headlights **warm white**, taillights **red**, license
     plates **white** (small inset boxes merged into the body, baked colours).
2. **Glass** — the greenhouse band (windshield, side windows, rear window) as
   a separate merged geometry rendered with its own material (see Materials).
   Slightly inset from the body so it never z-fights.

**Wheels** are NOT merged into the body. One shared wheel geometry (low-poly
cylinder, ~12 segments, axis along x): rim disc baked light grey, tire ring
baked near-black, via vertex colours.

### Variants (6 — "alle Autos")

| Variant   | Length | Character |
|-----------|--------|-----------|
| sedan     | 4.5 m  | hood + cabin + trunk step |
| hatchback | 3.9 m  | short hood, sloped hatch tail |
| wagon     | 4.6 m  | sedan front, long flat roof to the tail |
| suv       | 4.6 m  | tall body, high wheel line, upright tail |
| van       | 5.2 m  | Sprinter silhouette: short nose, tall long box, high roof |
| pickup    | 5.0 m  | cab + open low bed |

Each variant declares its **wheel layout**: wheelbase, track width, wheel
radius (van/SUV/pickup slightly larger). Variant + colour selection keeps the
existing stable id-hash (`carVariantForId`, `carColorForId`) — a vehicle keeps
its silhouette and colour for its wire lifetime. Palette stays 14 entries,
re-tuned toward the screenshot saturation.

### Materials

- **Paint**: `MeshPhysicalMaterial`, `vertexColors: true`, white base,
  `clearcoat ≈ 1.0`, `clearcoatRoughness ≈ 0.15`, `roughness ≈ 0.45`,
  `metalness ≈ 0.1` — glossy CS paint that picks up the scene environment
  (`scene.environment` cube RT exists via `look.ts`).
- **Glass**: separate `MeshPhysicalMaterial`, light sky-blue
  (`≈ 0x9fc8e8`), `roughness ≈ 0.05`, `metalness ≈ 0.4`,
  `envMapIntensity` high — bright reflective CS glass. No transparency
  (opaque like CS; avoids sorting cost at 2k+ instances).
- **Wheels**: reuse the paint material (vertex colours carry tire/rim); no
  per-instance tint (colour attribute written as white).

### Instancing & per-frame update (`carLayer.ts`)

Per variant: **body InstancedMesh + glass InstancedMesh** (share the same
matrix values; written twice). One global **wheel InstancedMesh** shared by
all variants (capacity `4 × CAR_CAPACITY`).

Per vehicle per frame (existing dead-reckon path unchanged —
`poseAtBlended` still supplies x/z/yaw):

1. Compose body matrix from pose + ground sample + carriage lift (as today);
   write into body AND glass mesh of the vehicle's variant.
2. **Wheel spin**: per-id persistent state `{ theta, lastTick, lastYaw }`.
   `theta += v · dt / wheelRadius` with `dt = (nowTick − lastTick) · SIM_DT`
   (dead-reckoned `v` from `VehKinematics`; clamped dt ≥ 0). Rolls forward and
   backward correctly.
3. **Steering**: front-wheel yaw offset `steer = clamp(k · Δyaw/dt)`, capped
   at ±0.45 rad, low-pass-filtered so it doesn't jitter. Rear wheels: no steer.
4. For each of the 4 wheel offsets of the variant's layout: wheel matrix =
   body matrix × translate(offset) × rotY(steer, front only) × rotX(theta),
   scaled by the variant's wheel radius. Written with the same scratch-object,
   no-allocation discipline as today.
5. Per-id maps pruned with the existing `CAR_CAPACITY * 2` sweep (extended to
   the new spin state).

Draw calls: 6 body + 6 glass + 1 wheels = **13 instanced draws** (previously
3). Matrix writes ≤ 6 per vehicle (1 body + 1 glass + 4 wheels) at ~2.4 k
visible cars ≈ 14 k matrix composes/frame — trivially cheap next to the
existing 10 k-agent pipeline. `frustumCulled = false` on all meshes (same
rationale as today).

### Far-LOD impostor (`flowLayer.ts`)

`buildCarGeometry()` keeps its signature and returns the new **sedan body
with 4 statically merged wheels and merged glass** (single geometry — at flow
LOD distances nothing rotates visibly). flowLayer itself is untouched.

## Error handling

- Geometry builders throw on merge failure (existing `mergeParts` behaviour).
- Wheel-state map missing an id → initialised lazily on first sight (same
  pattern as variant/colour maps).
- `dt ≤ 0` (tick replay/reorder) → skip spin increment, never NaN.

## Testing

1. **Unit (vitest, no GPU)**: variant table has 6 entries with distinct
   lengths & wheel layouts; hash stability unchanged for variant/colour;
   wheel-spin accumulation (θ advances by v·dt/r, clamps negative dt);
   steering clamp/decay; geometry builders produce non-empty indexed
   geometries with colour attributes; wheel offsets sit at ±track/2, ±wheelbase/2.
2. **Browser smoke (mandatory per CLAUDE.md)**: adapt the diorama smoke to
   verify (a) car meshes for all 6 variants + wheel mesh exist in the scene
   with count > 0 while traffic is live, (b) a sampled wheel instance's
   rotation matrix actually changes between two frames (wheels turn), and
   capture a screenshot for visual comparison against the reference.
   Requires the world bake (`geo:fetch` → `geo:bake-world` → symlink) per
   memory note in a fresh worktree.

## Rollout

Single PR to `main`. No data migration, no server deploy. Frontend CI gate
(typecheck, vitest, build) + browser smoke before merge.
