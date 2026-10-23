import { InlineNotification, Tag } from '@carbon/react'
import type { SubmitReport } from '../../api/types'
import { controlNumber } from '../../lib/format'

export type LiveState = 'posting' | 'submitted' | 'failed'
export interface Live {
  state: LiveState
  httpStatus?: number
  error?: string
}

const OUTCOME_TAG = {
  submitted: { type: 'green', text: 'Submitted' },
  failed: { type: 'red', text: 'Failed' },
  not_run: { type: 'cool-gray', text: 'Not run' },
} as const

const LIVE_TAG = {
  posting: { type: 'blue', text: 'Posting' },
  submitted: { type: 'green', text: 'Submitted' },
  failed: { type: 'red', text: 'Failed' },
} as const

/** Per-document results: live while the run is going, final from the report. */
export function SubmitResults({
  codes,
  live,
  report,
}: {
  codes: string[]
  live: Record<string, Live>
  report: SubmitReport | null | undefined
}) {
  return (
    <div className="submit-results">
      <h3>Submission results</h3>
      {report?.cancelled && (
        <InlineNotification
          className="wide-notification"
          kind="warning"
          lowContrast
          hideCloseButton
          title="Run cancelled"
          subtitle="Documents after the cancellation point were not sent."
        />
      )}
      <ul>
        {codes.map((code) => {
          const final = report?.documents.find((d) => d.transaction_code === code)
          const l = live[code]
          const tag = final ? OUTCOME_TAG[final.outcome] : l ? LIVE_TAG[l.state] : null
          const error = final?.error ?? l?.error
          return (
            <li key={code}>
              <strong>{code}</strong>
              {tag ? <Tag type={tag.type}>{tag.text}</Tag> : <Tag type="cool-gray">Waiting</Tag>}
              <span className="muted">
                {final?.isa_control != null && `ISA ${controlNumber(final.isa_control)} · GS ${final.gs_control}`}
                {(final?.http_status ?? l?.httpStatus) != null && ` · HTTP ${final?.http_status ?? l?.httpStatus}`}
                {final?.duration_ms != null && ` · ${final.duration_ms} ms`}
              </span>
              {error && <p className="row-error">{error}</p>}
            </li>
          )
        })}
      </ul>
      {report && Object.keys(report.trace_values).length > 0 && (
        <p className="muted">
          Linked by{' '}
          {Object.entries(report.trace_values)
            .map(([k, v]) => `${k}=${v}`)
            .join(' · ')}
        </p>
      )}
    </div>
  )
}
