import { useEffect, useRef, useState } from 'react'
import { getIgnoreAreas, setIgnoreAreas, type Area } from '../lib/api'
import { areaBetween, bigEnough } from '../lib/areas'

export interface IgnoreState {
  /** the areas saved on the camera, or null until it answers or when it has no such feature */
  saved: Area[] | null
  max: number
  editing: boolean
  /** what is drawn while editing */
  draft: Area[]
  saving: boolean
  error: string
  edit: () => void
  cancel: () => void
  add: (a: Area) => void
  undo: () => void
  clear: () => void
  save: () => void
}

export function useIgnoreAreas(initData: string): IgnoreState {
  const [saved, setSaved] = useState<Area[] | null>(null)
  const [max, setMax] = useState(20)
  const [editing, setEditing] = useState(false)
  const [draft, setDraft] = useState<Area[]>([])
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState('')

  useEffect(() => {
    if (!initData) return // not let in (yet)
    const abort = new AbortController()
    getIgnoreAreas(initData, abort.signal).then((r) => {
      if (abort.signal.aborted || !r) return
      setSaved(r.areas)
      setMax(r.max)
    })
    return () => abort.abort()
  }, [initData])

  return {
    saved,
    max,
    editing,
    draft,
    saving,
    error,
    edit: () => {
      setDraft(saved ?? [])
      setError('')
      setEditing(true)
    },
    cancel: () => {
      setEditing(false)
      setError('')
    },
    add: (a) => setDraft((d) => (d.length < max ? [...d, a] : d)),
    undo: () => setDraft((d) => d.slice(0, -1)),
    clear: () => setDraft([]),
    save: () => {
      setSaving(true)
      setError('')
      setIgnoreAreas(initData, draft)
        .then((r) => {
          setSaved(r.areas)
          setEditing(false)
        })
        .catch((e: Error) => setError(`Not saved: ${e.message}`))
        .finally(() => setSaving(false))
    },
  }
}

/** The ignored areas over the picture; while editing, dragging on it draws a new one. */
export function AreaLayer({ state }: { state: IgnoreState }) {
  const svg = useRef<SVGSVGElement>(null)
  const [drag, setDrag] = useState<{ x: number; y: number; to: { x: number; y: number } } | null>(null)
  const areas = state.editing ? state.draft : (state.saved ?? [])
  if (areas.length === 0 && !state.editing) return null

  const point = (e: React.PointerEvent) => {
    const r = svg.current!.getBoundingClientRect()
    return { x: (e.clientX - r.left) / r.width, y: (e.clientY - r.top) / r.height }
  }
  const drawing = drag && areaBetween(drag.x, drag.y, drag.to.x, drag.to.y)

  return (
    <svg
      ref={svg}
      className={state.editing ? 'areas editing' : 'areas'}
      viewBox="0 0 1 1"
      preserveAspectRatio="none"
      onPointerDown={(e) => {
        if (!state.editing) return
        e.currentTarget.setPointerCapture(e.pointerId)
        const p = point(e)
        setDrag({ ...p, to: p })
      }}
      onPointerMove={(e) => drag && setDrag({ ...drag, to: point(e) })}
      onPointerUp={(e) => {
        if (!drag) return
        const a = areaBetween(drag.x, drag.y, point(e).x, point(e).y)
        if (bigEnough(a)) state.add(a)
        setDrag(null)
      }}
      onPointerCancel={() => setDrag(null)}
    >
      {areas.map((a, i) => (
        <rect key={i} className="area" x={a.x} y={a.y} width={a.width} height={a.height} />
      ))}
      {drawing && <rect className="area drawing" x={drawing.x} y={drawing.y} width={drawing.width} height={drawing.height} />}
    </svg>
  )
}

export function IgnoreControls({ state }: { state: IgnoreState }) {
  if (state.saved === null) return null
  const n = state.editing ? state.draft.length : state.saved.length
  return (
    <section>
      <div className="head">
        <h2>Ignored areas</h2>
        <span className="status">{n === 0 ? 'none' : n === 1 ? '1 area' : `${n} areas`}</span>
      </div>
      {state.editing ? (
        <>
          <p className="status">
            Drag on the live view to cover what should not trigger alarms (a tree, a street…). {state.draft.length >= state.max && `At most ${state.max} areas.`}
          </p>
          <div className="actions">
            <button disabled={state.draft.length === 0 || state.saving} onClick={state.undo}>
              Undo
            </button>
            <button disabled={state.draft.length === 0 || state.saving} onClick={state.clear}>
              Clear
            </button>
            <button disabled={state.saving} onClick={state.cancel}>
              Cancel
            </button>
            <button disabled={state.saving} onClick={state.save}>
              {state.saving ? 'Saving…' : 'Save'}
            </button>
          </div>
        </>
      ) : (
        <>
          <p className="status">Motion inside these areas (shown in red on the live view) does not set off alarms. The live view, GIFs and snapshots stay whole.</p>
          <div className="actions">
            <button onClick={state.edit}>{n === 0 ? 'Draw areas' : 'Edit areas'}</button>
          </div>
        </>
      )}
      {state.error && <p className="status">{state.error}</p>}
    </section>
  )
}
