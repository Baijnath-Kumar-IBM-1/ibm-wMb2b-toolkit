import { useEffect, useRef, useState } from 'react'
import { api, unwrap } from '../../api/client'
import type { Party, PreviewReport } from '../../api/types'

export interface PreviewInput {
  environmentId: string | undefined
  packId: string | undefined
  codes: string[]
  p1: Party | null
  p2: Party | null
  overrides: Record<string, string>
}

/**
 * Renders the selected documents on the server (nothing is sent and no
 * control numbers are used). Re-runs shortly after the input stops changing.
 */
export function usePreview(input: PreviewInput) {
  const [report, setReport] = useState<PreviewReport | null>(null)
  const [error, setError] = useState<Error | null>(null)
  const [loading, setLoading] = useState(false)
  const [refreshKey, setRefreshKey] = useState(0)
  const seq = useRef(0)

  const { environmentId, packId, codes, p1, p2, overrides } = input
  const key = JSON.stringify([environmentId, packId, codes, p1, p2, overrides, refreshKey])

  useEffect(() => {
    const id = ++seq.current
    if (!environmentId || !packId || !p1 || !p2 || codes.length === 0) {
      setReport(null)
      setError(null)
      setLoading(false)
      return
    }
    setLoading(true)
    const timer = window.setTimeout(async () => {
      try {
        const next = await unwrap(
          api.POST('/preview', {
            body: {
              environment_id: environmentId,
              pack_id: packId,
              codes,
              p1,
              p2,
              trace_overrides: overrides,
            },
          }),
        )
        if (id !== seq.current) return
        setReport(next)
        setError(null)
      } catch (e) {
        if (id !== seq.current) return
        setReport(null)
        setError(e instanceof Error ? e : new Error('Preview failed'))
      } finally {
        if (id === seq.current) setLoading(false)
      }
    }, 350)
    return () => window.clearTimeout(timer)
    // `key` captures every input by value.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key])

  return { report, error, loading, refresh: () => setRefreshKey((k) => k + 1) }
}
