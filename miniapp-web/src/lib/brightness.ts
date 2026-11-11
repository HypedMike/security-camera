/** `value` moved by `steps` steps (negative to darken), kept inside min..max. */
export function nudge(value: number, steps: number, min: number, max: number, step: number): number {
  return Math.min(max, Math.max(min, value + steps * step))
}
