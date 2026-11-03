import { useEffect, useRef, useState } from 'react'
import { apiUrl } from '../lib/api'
import { appendPoint, MAX_POINTS, yMax, type Point } from '../lib/graph'

function usePoints(initData: string) {
  const [points, setPoints] = useState<Point[]>([])
  const [status, setStatus] = useState('connecting…')

  useEffect(() => {
    const es = new EventSource(apiUrl('/events', initData))
    es.onopen = () => setStatus('live')
    es.onerror = () => setStatus('reconnecting…')
    es.onmessage = (e) => setPoints((prev) => appendPoint(prev, JSON.parse(e.data) as Point))
    return () => es.close()
  }, [initData])

  return { points, status }
}

function cssVar(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim()
}

function draw(canvas: HTMLCanvasElement, points: Point[]) {
  const ctx = canvas.getContext('2d')
  if (!ctx) return
  const dpr = window.devicePixelRatio || 1
  const w = canvas.clientWidth
  const h = canvas.clientHeight
  canvas.width = w * dpr
  canvas.height = h * dpr
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
  ctx.clearRect(0, 0, w, h)

  const left = 44
  const top = 8
  const bottom = 4
  const max = yMax(points)
  const y = (v: number) => top + (h - top - bottom) * (1 - v / max)
  const x = (i: number) => left + (w - left) * (i / (MAX_POINTS - 1))

  ctx.font = '11px system-ui'
  ctx.fillStyle = cssVar('--muted')
  ctx.strokeStyle = cssVar('--grid')
  ctx.lineWidth = 1
  for (let i = 0; i <= 4; i++) {
    const v = (max * i) / 4
    const yy = Math.round(y(v)) + 0.5
    ctx.beginPath()
    ctx.moveTo(left, yy)
    ctx.lineTo(w, yy)
    ctx.stroke()
    ctx.fillText(v.toFixed(3), 2, yy + 4)
  }
  if (!points.length) return

  ctx.strokeStyle = cssVar('--accent')
  ctx.lineWidth = 2
  ctx.lineJoin = 'round'
  ctx.beginPath()
  const offset = MAX_POINTS - points.length
  points.forEach((p, i) => (i ? ctx.lineTo(x(offset + i), y(p[1])) : ctx.moveTo(x(offset + i), y(p[1]))))
  ctx.stroke()
}

export function MotionGraph({ initData }: { initData: string }) {
  const { points, status } = usePoints(initData)
  const canvas = useRef<HTMLCanvasElement>(null)

  useEffect(() => {
    const el = canvas.current
    if (!el) return
    const redraw = () => draw(el, points)
    const frame = requestAnimationFrame(redraw)
    window.addEventListener('resize', redraw)
    return () => {
      cancelAnimationFrame(frame)
      window.removeEventListener('resize', redraw)
    }
  }, [points])

  const last = points.at(-1)
  return (
    <section>
      <div className="head">
        <h2>Motion score</h2>
        <span className="status">{status}</span>
      </div>
      <div className="value">{last ? last[1].toFixed(4) : '–'}</div>
      <canvas ref={canvas} />
    </section>
  )
}
