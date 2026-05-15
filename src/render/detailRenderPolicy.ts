export type DetailRenderCandidate = {
  category: string;
  assetCategory?: string;
};

export function shouldRenderDetail(detail: DetailRenderCandidate): boolean {
  return (
    detail.category !== 'field' &&
    detail.assetCategory !== 'farm-field' &&
    detail.assetCategory !== 'station-roof' &&
    detail.assetCategory !== 'rail-depot' &&
    detail.assetCategory !== 'road-stop'
  );
}
