/** A point of the motion score: [unix ms, score]. */
export type Point = [number, number]

/** How many points the graph keeps and shows. The server sends the same history. */
export const MAX_POINTS = 300

/** Returns points with p added, keeping only the latest MAX_POINTS. */
export function appendPoint(points: Point[], p: Point): Point[] {
  const next = [...points, p]
  return next.length > MAX_POINTS ? next.slice(next.length - MAX_POINTS) : next
}

/** The top of the y axis: the largest score with some room, never below 0.005. */
export function yMax(points: Point[]): number {
  return Math.max(0.005, ...points.map((p) => p[1])) * 1.1
}
