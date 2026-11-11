import { describe, expect, it } from 'vitest'
import { nudge } from './brightness'

describe('nudge', () => {
  it('moves by whole steps', () => {
    expect(nudge(100, 1, 20, 400, 10)).toBe(110)
    expect(nudge(100, -3, 20, 400, 10)).toBe(70)
  })
  it('stays inside the limits', () => {
    expect(nudge(25, -1, 20, 400, 10)).toBe(20)
    expect(nudge(395, 1, 20, 400, 10)).toBe(400)
  })
})
