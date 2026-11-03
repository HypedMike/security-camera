import { useCallback, useEffect, useRef, useState } from 'react'
import { apiUrl } from '../lib/api'

// The video stops after a while and when the app is hidden: nobody is
// watching, and every open stream costs the camera machine CPU and bandwidth.
const STREAM_MINUTES = 10

export function LiveView({ initData }: { initData: string }) {
  // src is empty while the video is stopped; why, if it is, is in reason
  const [src, setSrc] = useState('')
  const [reason, setReason] = useState('')
  const timer = useRef<number>(undefined)

  const stop = useCallback((why: string) => {
    window.clearTimeout(timer.current)
    setSrc('')
    setReason(why)
  }, [])

  const start = useCallback(() => {
    setReason('')
    // the time makes every start a new request
    setSrc(`${apiUrl('/stream', initData)}&t=${Date.now()}`)
    window.clearTimeout(timer.current)
    timer.current = window.setTimeout(() => stop(`Paused after ${STREAM_MINUTES} minutes.`), STREAM_MINUTES * 60_000)
  }, [initData, stop])

  useEffect(() => {
    const onVisibility = () => (document.hidden ? stop('Paused while the app is hidden.') : start())
    document.addEventListener('visibilitychange', onVisibility)
    start()
    return () => {
      document.removeEventListener('visibilitychange', onVisibility)
      window.clearTimeout(timer.current)
    }
  }, [start, stop])

  return (
    <section>
      <div className="head">
        <h2>Live view</h2>
        <span className="status">{src ? 'live' : 'paused'}</span>
      </div>
      <div className="video">
        {src && <img src={src} alt="Live camera view" onError={() => stop('The video is not available right now (too many viewers?).')} />}
        {!src && (
          <div className="overlay">
            <span>{reason}</span>
            <button onClick={start}>Resume</button>
          </div>
        )}
      </div>
    </section>
  )
}
