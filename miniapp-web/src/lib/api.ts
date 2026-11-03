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
