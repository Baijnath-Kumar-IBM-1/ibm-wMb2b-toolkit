import { useCallback, useEffect, useRef, useState } from 'react'
import { api, ApiFailure, unwrap } from '../api/client'
import type { Run } from '../api/types'

const POLL_MS = 700

export interface RunHandle {
  run: Run | null
  /** True from the moment the request is sent until the run has ended. */
  running: boolean
  error: ApiFailure | null
  start: (begin: () => Promise<{ run_id: string }>) => Promise<void>
  cancel: () => Promise<void>
  reset: () => void
}

/**
 * Starts a long operation (`202 {run_id}`) and follows `GET /runs/{id}` until
 * it ends. `onDone` receives the final run, including its report.
 */
export function useRun(onDone?: (run: Run) => void): RunHandle {
  const [run, setRun] = useState<Run | null>(null)
  const [starting, setStarting] = useState(false)
  const [error, setError] = useState<ApiFailure | null>(null)
  const generation = useRef(0)
  const done = useRef(onDone)
  done.current = onDone

  useEffect(
    () => () => {
      generation.current++
    },
    [],
  )

  const start = useCallback(async (begin: () => Promise<{ run_id: string }>) => {
    const mine = ++generation.current
    setError(null)
    setRun(null)
    setStarting(true)
    try {
      const { run_id } = await begin()
      for (;;) {
        const current = await unwrap(api.GET('/runs/{id}', { params: { path: { id: run_id } } }))
        if (mine !== generation.current) return
        setRun(current)
        if (current.status !== 'running') {
          setStarting(false)
          done.current?.(current)
          return
        }
        await new Promise((resolve) => setTimeout(resolve, POLL_MS))
        if (mine !== generation.current) return
      }
    } catch (e) {
      if (mine !== generation.current) return
      setError(e instanceof ApiFailure ? e : new ApiFailure(0, undefined))
      setStarting(false)
    }
  }, [])

  const cancel = useCallback(async () => {
    if (!run || run.status !== 'running') return
    try {
      await unwrap(api.POST('/runs/{id}/cancel', { params: { path: { id: run.id } } }))
    } catch (e) {
      setError(e instanceof ApiFailure ? e : new ApiFailure(0, undefined))
    }
  }, [run])

  const reset = useCallback(() => {
    generation.current++
    setRun(null)
    setError(null)
    setStarting(false)
  }, [])

  return { run, running: starting, error, start, cancel, reset }
}
