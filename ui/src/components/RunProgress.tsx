import { Button, InlineLoading } from '@carbon/react'
import type { RunHandle } from '../lib/useRun'

/** Shows a run in progress with a Cancel button, or the error that stopped it. */
export function RunProgress({ handle, label }: { handle: RunHandle; label: string }) {
  if (!handle.running) return null
  return (
    <div className="run-progress" role="status" aria-live="polite">
      <InlineLoading description={label} />
      <Button kind="ghost" size="sm" onClick={() => void handle.cancel()}>
        Cancel
      </Button>
    </div>
  )
}
