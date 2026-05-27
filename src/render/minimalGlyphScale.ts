export type ScreenStableWorldSizeOptions = {
  minWorld?: number;
  maxWorld?: number;
};

export function screenStableWorldSize(
  screenPixels: number,
  cameraScale: number,
  options: ScreenStableWorldSizeOptions = {},
): number {
  const minWorld = options.minWorld ?? 10;
  const maxWorld = options.maxWorld ?? 48;
  const scale = Math.max(0.001, cameraScale);
  return Math.max(minWorld, Math.min(maxWorld, screenPixels / scale));
}
