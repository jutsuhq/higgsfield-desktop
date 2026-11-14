/**
 * The zoom and pan both images of the image compare follow (`ImageCompareState` in `Compare.swift`). One state drives
 * both panes, so they stay in step; each pane shows the same magnification around its own center, panning together
 * from there. It needs the pane and image sizes only the window knows, so it lives here, out of the components.
 */
export interface ImageCompareState {
  /** How many image pixels a point shows: 1 is actual size. */
  zoom: number;
  /** Points panned from the centered image, at the current zoom. */
  offsetX: number;
  offsetY: number;
  /** Whether each pane fits its image (`true`) or shows actual pixels with panning (`false`). */
  fitsPane: boolean;
}

export const minZoom = 0.1;
export const maxZoom = 8;

/** Each image fitted into its pane, centered. */
export const fitState: ImageCompareState = { zoom: 1, offsetX: 0, offsetY: 0, fitsPane: true };

/** Actual size, centered: one image pixel per point, so different sizes compare at the same magnification. */
export const actualSizeState: ImageCompareState = { zoom: 1, offsetX: 0, offsetY: 0, fitsPane: false };

/** How an image scales to fit a pane, never magnifying past actual size. */
export function fitScale(imageWidth: number, imageHeight: number, paneWidth: number, paneHeight: number): number {
  if (imageWidth <= 0 || imageHeight <= 0 || paneWidth <= 0 || paneHeight <= 0) return 1;
  return Math.min(paneWidth / imageWidth, paneHeight / imageHeight, 1);
}

/**
 * The scale a pane draws its image at: the fit scale magnified while fitting, actual pixels magnified otherwise. Zoom
 * 1 in actual size is 1:1, whatever the pane holds.
 */
export function displayScale(state: ImageCompareState, fit: number): number {
  return state.fitsPane ? fit * state.zoom : state.zoom;
}

export function withZoom(state: ImageCompareState, zoom: number): ImageCompareState {
  return { ...state, zoom: Math.min(Math.max(zoom, minZoom), maxZoom) };
}

export function panned(state: ImageCompareState, dx: number, dy: number): ImageCompareState {
  return { ...state, offsetX: state.offsetX + dx, offsetY: state.offsetY + dy };
}

/** Fit is off while each image already fits at zoom 1. */
export function isFit(state: ImageCompareState): boolean {
  return state.fitsPane && state.zoom === 1;
}

/** Actual Size is off while the images already show at 100%. */
export function isActualSize(state: ImageCompareState): boolean {
  return !state.fitsPane && state.zoom === 1;
}
