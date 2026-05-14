export type SpritePixelBuffer = {
  data: Uint8ClampedArray;
  width: number;
  height: number;
  path: string;
};

export function cleanupSpritePixels(buffer: SpritePixelBuffer): void {
  for (let i = 0; i < buffer.data.length; i += 4) {
    const r = buffer.data[i];
    const g = buffer.data[i + 1];
    const b = buffer.data[i + 2];
    if (isTransparentSourcePixel(r, g, b)) clearPixel(buffer.data, i);
  }

  if (hasShapeMetadataRow(buffer.path)) clearRow(buffer.data, buffer.width, buffer.height, 0);
}

export function isTransparentSourcePixel(r: number, g: number, b: number): boolean {
  return (b > 190 && r < 45 && g < 80) || (r > 248 && g > 248 && b > 248) || (r > 220 && g > 248 && b > 248);
}

function hasShapeMetadataRow(path: string): boolean {
  return path.endsWith('_shape.png');
}

function clearRow(data: Uint8ClampedArray, width: number, height: number, y: number): void {
  if (y < 0 || y >= height) return;
  for (let x = 0; x < width; x += 1) clearPixel(data, (y * width + x) * 4);
}

function clearPixel(data: Uint8ClampedArray, offset: number): void {
  data[offset] = 0;
  data[offset + 1] = 0;
  data[offset + 2] = 0;
  data[offset + 3] = 0;
}
