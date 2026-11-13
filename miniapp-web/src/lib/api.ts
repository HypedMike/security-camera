// Every request carries the initData Telegram gave the app: the server checks
// its signature and that the user is allowed in. It is in the query string
// because <img> and EventSource can't set headers.
export function apiUrl(path: string, initData: string): string {
  return `${path}?auth=${encodeURIComponent(initData)}`
}

export type Access = 'allowed' | 'denied' | 'unreachable' | 'error'

export async function checkAccess(initData: string, signal?: AbortSignal): Promise<Access> {
  try {
    const res = await fetch(apiUrl('/api/check', initData), { signal })
    if (res.status === 401) return 'denied'
    return res.ok ? 'allowed' : 'error'
  } catch {
    return 'unreachable'
  }
}

export interface Brightness {
  /** percent, 100 is the camera as it is */
  value: number
  min: number
  max: number
  step: number
}

/** The brightness setting, or null when the camera has no such control. */
export async function getBrightness(initData: string, signal?: AbortSignal): Promise<Brightness | null> {
  try {
    const res = await fetch(apiUrl('/api/brightness', initData), { signal })
    return res.ok ? ((await res.json()) as Brightness) : null
  } catch {
    return null
  }
}

/** Sets the brightness (the camera saves it in its preferences) and returns the value in use. */
export async function setBrightness(initData: string, value: number, signal?: AbortSignal): Promise<Brightness> {
  const res = await fetch(`${apiUrl('/api/brightness', initData)}&value=${value}`, { method: 'POST', signal })
  if (!res.ok) throw new Error((await res.text()).trim() || `HTTP ${res.status}`)
  return (await res.json()) as Brightness
}

/** A rectangle of the picture, as fractions of its width and height. */
export interface Area {
  x: number
  y: number
  width: number
  height: number
}

export interface IgnoreAreas {
  areas: Area[]
  max: number
}

/** The areas where motion is ignored, or null when the camera has no such feature. */
export async function getIgnoreAreas(initData: string, signal?: AbortSignal): Promise<IgnoreAreas | null> {
  try {
    const res = await fetch(apiUrl('/api/ignore', initData), { signal })
    return res.ok ? ((await res.json()) as IgnoreAreas) : null
  } catch {
    return null
  }
}

/** Replaces the ignored areas (the camera saves them in its preferences) and returns the ones in use. */
export async function setIgnoreAreas(initData: string, areas: Area[], signal?: AbortSignal): Promise<IgnoreAreas> {
  const res = await fetch(`${apiUrl('/api/ignore', initData)}&areas=${encodeURIComponent(JSON.stringify(areas))}`, {
    method: 'POST',
    signal,
  })
  if (!res.ok) throw new Error((await res.text()).trim() || `HTTP ${res.status}`)
  return (await res.json()) as IgnoreAreas
}
