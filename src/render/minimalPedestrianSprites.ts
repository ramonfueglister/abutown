export type MinimalPedestrianSheetName = 'minimal-pedestrian';
export type MinimalPedestrianKind = 'pedestrian';

export type MinimalPedestrianSprite = {
  sheet: MinimalPedestrianSheetName;
  variantIndex: number;
  kind: MinimalPedestrianKind;
  scale: number;
};

export function candidateMinimalPedestrianSprites(): MinimalPedestrianSprite[] {
  return Array.from({ length: 8 }, (_, variantIndex) => ({
    sheet: 'minimal-pedestrian' as const,
    variantIndex,
    kind: 'pedestrian' as const,
    scale: 1,
  }));
}
