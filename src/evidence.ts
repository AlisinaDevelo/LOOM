export function compactPath(source: string, maxLength = 72): string {
  if (source.length <= maxLength) return source;
  if (maxLength <= 1) return "…".slice(0, maxLength);
  const headLength = Math.min(14, Math.max(1, Math.floor((maxLength - 1) / 3)));
  const tailLength = Math.max(0, maxLength - headLength - 1);
  const tail = tailLength > 0 ? source.slice(-tailLength) : "";
  return `${source.slice(0, headLength)}…${tail}`;
}

export type ImageRegionAnchor = {
  x: number;
  y: number;
  width: number;
  height: number;
  image_width: number;
  image_height: number;
};

export type ProjectedImageRegion = {
  leftPercent: number;
  topPercent: number;
  widthPercent: number;
  heightPercent: number;
  canvasWidth: number;
  canvasHeight: number;
  rotation: 0 | 90 | 180 | 270;
  scale: number;
};

/**
 * Projects LOOM's oriented top-left pixel anchor into a viewer canvas.
 *
 * OCR coordinates are stored after EXIF orientation, so this function only applies the viewer's
 * explicit quarter-turn and device scale. Percentages keep the overlay stable when the stage is
 * resized; canvas dimensions retain enough information for deterministic HiDPI tests.
 */
export function projectImageRegion(
  anchor: ImageRegionAnchor,
  zoom = 1,
  rotation: number = 0,
  deviceScale = 1,
): ProjectedImageRegion {
  const normalizedRotation = (((Math.round(rotation / 90) * 90) % 360) + 360) % 360 as
    | 0
    | 90
    | 180
    | 270;
  const boundedZoom = Math.max(0.25, Math.min(4, zoom));
  const boundedDeviceScale = Math.max(0.5, Math.min(4, deviceScale));
  const x = Math.max(0, Math.min(anchor.x, anchor.image_width));
  const y = Math.max(0, Math.min(anchor.y, anchor.image_height));
  const width = Math.max(0, Math.min(anchor.width, anchor.image_width - x));
  const height = Math.max(0, Math.min(anchor.height, anchor.image_height - y));
  const { left, top, regionWidth, regionHeight, canvasWidth, canvasHeight } =
    normalizedRotation === 0
      ? {
          left: x,
          top: y,
          regionWidth: width,
          regionHeight: height,
          canvasWidth: anchor.image_width,
          canvasHeight: anchor.image_height,
        }
      : normalizedRotation === 90
        ? {
            left: anchor.image_height - (y + height),
            top: x,
            regionWidth: height,
            regionHeight: width,
            canvasWidth: anchor.image_height,
            canvasHeight: anchor.image_width,
          }
        : normalizedRotation === 180
          ? {
              left: anchor.image_width - (x + width),
              top: anchor.image_height - (y + height),
              regionWidth: width,
              regionHeight: height,
              canvasWidth: anchor.image_width,
              canvasHeight: anchor.image_height,
            }
          : {
              left: y,
              top: anchor.image_width - (x + width),
              regionWidth: height,
              regionHeight: width,
              canvasWidth: anchor.image_height,
              canvasHeight: anchor.image_width,
            };
  const scale = boundedZoom * boundedDeviceScale;
  return {
    leftPercent: (left / canvasWidth) * 100,
    topPercent: (top / canvasHeight) * 100,
    widthPercent: (regionWidth / canvasWidth) * 100,
    heightPercent: (regionHeight / canvasHeight) * 100,
    canvasWidth: canvasWidth * scale,
    canvasHeight: canvasHeight * scale,
    rotation: normalizedRotation,
    scale,
  };
}
