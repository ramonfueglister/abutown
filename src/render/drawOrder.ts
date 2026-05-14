export type SceneDrawableType = 'rail' | 'road' | 'railStation' | 'tree' | 'building' | 'car';

export function drawPassForType(type: SceneDrawableType): number {
  if (type === 'rail' || type === 'road') return 0;
  if (type === 'car') return 1;
  return 2;
}
