import type { Schemas } from './client'
import { getToken } from './token'

export type ServerEvent = Schemas['Event']

/**
 * Follows `/api/events`. `EventSource` cannot send an Authorization header,
 * so the stream is read with `fetch`. Reconnects with a short back-off until
 * the returned function is called.
 */
export function subscribeEvents(
  onEvent: (event: ServerEvent) => void,
  onLagged?: () => void,
): () => void {
  const abort = new AbortController()

  const run = async () => {
    let delay = 500
    while (!abort.signal.aborted) {
      try {
        const token = getToken()
        const res = await fetch('/api/events', {
          headers: token ? { Authorization: `Bearer ${token}` } : {},
          signal: abort.signal,
        })
        if (!res.ok || !res.body) throw new Error('stream unavailable')
        delay = 500
        const reader = res.body.pipeThrough(new TextDecoderStream()).getReader()
        let buffer = ''
        for (;;) {
          const { value, done } = await reader.read()
          if (done) break
          buffer += value
          let end
          while ((end = buffer.indexOf('\n\n')) >= 0) {
            handleFrame(buffer.slice(0, end), onEvent, onLagged)
            buffer = buffer.slice(end + 2)
          }
        }
      } catch {
        if (abort.signal.aborted) return
      }
      await new Promise((r) => setTimeout(r, delay))
      delay = Math.min(delay * 2, 10_000)
    }
  }
  void run()
  return () => abort.abort()
}

function handleFrame(frame: string, onEvent: (e: ServerEvent) => void, onLagged?: () => void) {
  let name = 'message'
  const data: string[] = []
  for (const line of frame.split('\n')) {
    if (line.startsWith('event:')) name = line.slice(6).trim()
    else if (line.startsWith('data:')) data.push(line.slice(5).trimStart())
  }
  if (name === 'lagged') onLagged?.()
  if (name !== 'event' || data.length === 0) return
  try {
    onEvent(JSON.parse(data.join('\n')) as ServerEvent)
  } catch {
    // Ignore a malformed frame; the next one is independent.
  }
}
