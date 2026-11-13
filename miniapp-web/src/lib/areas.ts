import type { Area } from './api'

/** The smallest side of a drawn area, as a fraction of the picture: a tap is not an area. */
export const MIN_SIDE = 0.02

const clamp01 = (v: number) => Math.min(1, Math.max(0, v))

/** The rectangle between two points (fractions of the picture), whichever way it was dragged, kept inside the picture. */
export function areaBetween(ax: number, ay: number, bx: number, by: number): Area {
  const [x0, x1] = [clamp01(Math.min(ax, bx)), clamp01(Math.max(ax, bx))]
  const [y0, y1] = [clamp01(Math.min(ay, by)), clamp01(Math.max(ay, by))]
  return { x: x0, y: y0, width: x1 - x0, height: y1 - y0 }
}

/** Whether a dragged area is big enough to keep. */
export function bigEnough(a: Area): boolean {
  return a.width >= MIN_SIDE && a.height >= MIN_SIDE
}
