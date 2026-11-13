import { useEffect, useState } from 'react'
import { BrightnessControl } from './components/BrightnessControl'
import { AreaLayer, IgnoreControls, useIgnoreAreas } from './components/IgnoreAreas'
import { LiveView } from './components/LiveView'
import { MotionGraph } from './components/MotionGraph'
import { checkAccess, type Access } from './lib/api'

const webApp = window.Telegram?.WebApp
const initData = webApp?.initData ?? ''

const messages: Record<Exclude<Access, 'allowed'>, string> = {
  denied: 'You are not allowed to see this camera, or the link has expired: close the app and open it again from the bot.',
  unreachable: 'The camera is not reachable.',
  error: 'Something went wrong.',
}

export function App() {
  const [access, setAccess] = useState<Access | 'checking'>('checking')

  useEffect(() => {
    if (!initData) return
    webApp?.ready()
    webApp?.expand()
    const abort = new AbortController()
    checkAccess(initData, abort.signal).then((a) => {
      if (!abort.signal.aborted) setAccess(a)
    })
    return () => abort.abort()
  }, [])

  const ignore = useIgnoreAreas(access === 'allowed' ? initData : '')

  if (!initData) return <p className="notice">Open this page from Telegram, with the /app command.</p>
  if (access === 'checking') return <p className="notice">Connecting…</p>
  if (access !== 'allowed') return <p className="notice">{messages[access]}</p>

  return (
    <main>
      <LiveView initData={initData}>
        <AreaLayer state={ignore} />
      </LiveView>
      <IgnoreControls state={ignore} />
      <BrightnessControl initData={initData} />
      <MotionGraph initData={initData} />
    </main>
  )
}
