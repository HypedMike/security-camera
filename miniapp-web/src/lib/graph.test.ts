import { describe, expect, it } from 'vitest'
import { appendPoint, MAX_POINTS, yMax, type Point } from './graph'

describe('appendPoint', () => {
  it('keeps only the latest points', () => {
    let points: Point[] = []
    for (let i = 0; i < MAX_POINTS + 10; i++) points = appendPoint(points, [i, i])
    expect(points).toHaveLength(MAX_POINTS)
    expect(points[0][0]).toBe(10)
    expect(points[points.length - 1][0]).toBe(MAX_POINTS + 9)
  })

  it('does not change the array it is given', () => {
    const points: Point[] = [[1, 1]]
    appendPoint(points, [2, 2])
    expect(points).toHaveLength(1)
  })
})

describe('yMax', () => {
  it('has a floor, so a quiet scene is not drawn as a huge signal', () => {
    expect(yMax([])).toBeCloseTo(0.0055)
    expect(yMax([[0, 0.001]])).toBeCloseTo(0.0055)
  })

  it('leaves room above the largest value', () => {
    expect(yMax([[0, 0.1], [1, 0.02]])).toBeCloseTo(0.11)
  })
})
