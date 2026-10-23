import { useCallback, useEffect, useRef, useState } from 'react'

export interface Loaded<T> {
  data: T | undefined
  /** True until the first result for the current inputs arrives. */
  loading: boolean
  error: Error | undefined
  /** Re-runs the loader without clearing what is on screen. */
  reload: () => Promise<void>
}

/**
 * Runs `load` whenever `deps` change and again on `reload()`. A slower,
 * older request never overwrites a newer one. Pass `null` to stay idle.
 */
export function useLoad<T>(load: (() => Promise<T>) | null, deps: readonly unknown[]): Loaded<T> {
  const [state, setState] = useState<{ data?: T; error?: Error; loading: boolean }>({
    loading: load !== null,
  })
  const loader = useRef(load)
  loader.current = load
  const seq = useRef(0)

  const run = useCallback(async (keep: boolean) => {
    const current = loader.current
    const id = ++seq.current
    if (!current) {
      setState({ loading: false })
      return
    }
    if (!keep) setState({ loading: true })
    try {
      const data = await current()
      if (id === seq.current) setState({ data, loading: false })
    } catch (e) {
      if (id === seq.current) {
        setState({ error: e instanceof Error ? e : new Error('Request failed'), loading: false })
      }
    }
  }, [])

  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(() => void run(false), deps)
  useEffect(
    () => () => {
      seq.current++
    },
    [],
  )

  const reload = useCallback(() => run(true), [run])
  return { data: state.data, loading: state.loading, error: state.error, reload }
}
