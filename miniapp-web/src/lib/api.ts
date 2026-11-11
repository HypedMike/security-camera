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
