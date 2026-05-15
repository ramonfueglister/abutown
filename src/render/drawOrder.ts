export type DrawableType = 'rail' | 'road' | 'railStation' | 'detail' | 'tree' | 'building' | 'train' | 'car' | 'pedestrian';

export type DrawableOrderInput = {
  type: DrawableType;
  isoY: number;
  x: number;
};

export function compareDrawableOrder(a: DrawableOrderInput, b: DrawableOrderInput): number {
  const flatActorOrder = compareFlatInfrastructureToActor(a.type, b.type);
  if (flatActorOrder !== 0) return flatActorOrder;

  return drawLayer(a.type) - drawLayer(b.type) ||
    a.isoY - b.isoY ||
    drawPriority(a.type) - drawPriority(b.type) ||
    a.x - b.x;
}

export function drawLayer(type: DrawableType): number {
  void type;
  return 0;
}

export function drawPriority(type: DrawableType): number {
  if (type === 'road') return 0;
  if (type === 'rail') return 1;
  if (type === 'railStation') return 2;
  if (type === 'train') return 3;
  if (type === 'car') return 4;
  if (type === 'pedestrian') return 5;
  if (type === 'detail') return 6;
  if (type === 'tree') return 7;
  return 7;
}

function compareFlatInfrastructureToActor(a: DrawableType, b: DrawableType): number {
  if (isFlatInfrastructure(a) && isActor(b)) return -1;
  if (isActor(a) && isFlatInfrastructure(b)) return 1;
  return 0;
}

function isFlatInfrastructure(type: DrawableType): boolean {
  return type === 'road' || type === 'rail' || type === 'railStation';
}

function isActor(type: DrawableType): boolean {
  return type === 'train' || type === 'car' || type === 'pedestrian';
}
