import { Button, CodeSnippet, InlineLoading, Modal, Tag } from '@carbon/react'
import { Copy, Download } from '@carbon/icons-react'
import { api, unwrap } from '../../api/client'
import type { Submission } from '../../api/types'
import { ErrorNotice } from '../../components/States'
import { controlNumber, downloadText, formatDateTime } from '../../lib/format'
import { useLoad } from '../../lib/useLoad'
import { RESULT_TAG } from './resultTag'

/** One submission in full, including the EDI payload that was sent. */
export function SubmissionDetail({ id, onClose }: { id: string; onClose: () => void }) {
  const detail = useLoad<Submission>(
    () => unwrap(api.GET('/submissions/{id}', { params: { path: { id } } })),
    [id],
  )
  const s = detail.data
  const tag = s ? RESULT_TAG[s.result] : null
  const traces = Object.entries(s?.trace_values ?? {})

  return (
    <Modal
      open
      passiveModal
      size="lg"
      modalHeading={s ? `${s.transaction_code} · ${s.domain}` : 'Submission'}
      modalLabel="Activity"
      onRequestClose={onClose}
    >
      {detail.loading && <InlineLoading description="Loading submission" />}
      {detail.error && <ErrorNotice error={detail.error} onRetry={() => void detail.reload()} />}
      {s && tag && (
        <div className="activity-detail">
          <p>
            <Tag type={tag.type}>{tag.text}</Tag>
            <span className="muted">
              {formatDateTime(s.submitted_at)} · ISA {controlNumber(s.isa_control)} · GS {s.gs_control}
              {s.http_status != null && ` · HTTP ${s.http_status}`}
              {s.duration_ms != null && ` · ${s.duration_ms} ms`}
            </span>
          </p>
          {traces.length > 0 && (
            <>
              <h3>Linked by</h3>
              <ul className="trace-list">
                {traces.map(([k, v]) => (
                  <li key={k}>
                    <code>{k}</code> = {String(v)}
                  </li>
                ))}
              </ul>
            </>
          )}
          {s.response_excerpt && (
            <>
              <h3>Response</h3>
              <CodeSnippet type="multi" feedback="Copied to clipboard" wrapText>
                {s.response_excerpt}
              </CodeSnippet>
            </>
          )}
          <div className="activity-payload-heading">
            <h3>EDI payload</h3>
            {s.payload && (
              <div className="preview-actions">
                <Button
                  kind="ghost"
                  size="sm"
                  renderIcon={Copy}
                  onClick={() => void navigator.clipboard?.writeText(s.payload ?? '')}
                >
                  Copy
                </Button>
                <Button
                  kind="ghost"
                  size="sm"
                  renderIcon={Download}
                  onClick={() => downloadText(`${s.transaction_code}-${controlNumber(s.isa_control)}.edi`, s.payload ?? '')}
                >
                  Download
                </Button>
              </div>
            )}
          </div>
          {s.payload ? (
            <CodeSnippet type="multi" feedback="Copied to clipboard" wrapText>
              {s.payload.split('~').join('~\n')}
            </CodeSnippet>
          ) : (
            <p className="muted">The payload was not stored for this submission.</p>
          )}
        </div>
      )}
    </Modal>
  )
}
