import { useEffect, useRef, useState } from 'react'
import { getBrightness, setBrightness, type Brightness } from '../lib/api'
import { nudge } from '../lib/brightness'

// A change is sent once the buttons or the slider rest for a moment: the
// camera saves it in its preferences file on every change.
const SEND_AFTER_MS = 300

export function BrightnessControl({ initData }: { initData: string }) {
  // null until the camera answers, or for good if it has no such control
  const [range, setRange] = useState<Brightness | null>(null)
  const [value, setValue] = useState(100)
  const [error, setError] = useState('')
  const timer = useRef<number>(undefined)

  useEffect(() => {
    const abort = new AbortController()
    getBrightness(initData, abort.signal).then((b) => {
      if (abort.signal.aborted || !b) return
      setRange(b)
      setValue(b.value)
    })
    return () => {
      abort.abort()
      window.clearTimeout(timer.current)
    }
  }, [initData])

  if (!range) return null

  const change = (next: number) => {
    setValue(next)
    setError('')
    window.clearTimeout(timer.current)
    timer.current = window.setTimeout(() => {
      setBrightness(initData, next)
        .then((b) => setValue(b.value))
        .catch((e: Error) => setError(`Not saved: ${e.message}`))
    }, SEND_AFTER_MS)
  }

  return (
    <section>
      <div className="head">
        <h2>Brightness</h2>
        <span className="status">{value === 100 ? 'as the camera' : value > 100 ? 'brighter' : 'darker'}</span>
      </div>
      <div className="brightness">
        <button aria-label="Darker" disabled={value <= range.min} onClick={() => change(nudge(value, -1, range.min, range.max, range.step))}>
          −
        </button>
        <input
          type="range"
          aria-label="Brightness"
          min={range.min}
          max={range.max}
          step={range.step}
          value={value}
          onChange={(e) => change(Number(e.target.value))}
        />
        <button aria-label="Brighter" disabled={value >= range.max} onClick={() => change(nudge(value, 1, range.min, range.max, range.step))}>
          +
        </button>
        <span className="value">{value}%</span>
      </div>
      {error && <p className="status">{error}</p>}
    </section>
  )
}
