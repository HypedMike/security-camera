import { describe, expect, it } from 'vitest'
import { areaBetween, bigEnough } from './areas'

describe('areaBetween', () => {
  it('is the same whichever way it is dragged', () => {
    const want = { x: 0.2, y: 0.1, width: 0.3, height: 0.4 }
    for (const [ax, ay, bx, by] of [
      [0.2, 0.1, 0.5, 0.5],
      [0.5, 0.5, 0.2, 0.1],
      [0.5, 0.1, 0.2, 0.5],
    ]) {
      const a = areaBetween(ax, ay, bx, by)
      expect(a.x).toBeCloseTo(want.x)
      expect(a.y).toBeCloseTo(want.y)
      expect(a.width).toBeCloseTo(want.width)
      expect(a.height).toBeCloseTo(want.height)
    }
  })

  it('stays inside the picture', () => {
    expect(areaBetween(-0.5, 0.5, 1.5, 2)).toEqual({ x: 0, y: 0.5, width: 1, height: 0.5 })
  })
})

describe('bigEnough', () => {
  it('drops taps and slivers', () => {
    expect(bigEnough({ x: 0, y: 0, width: 0, height: 0 })).toBe(false)
    expect(bigEnough({ x: 0, y: 0, width: 0.5, height: 0.01 })).toBe(false)
    expect(bigEnough({ x: 0, y: 0, width: 0.1, height: 0.1 })).toBe(true)
  })
})
