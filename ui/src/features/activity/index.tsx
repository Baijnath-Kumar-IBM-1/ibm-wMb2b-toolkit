import { useMemo, useState } from 'react'
import {
  Button,
  Select,
  SelectItem,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
  Tag,
  TextInput,
} from '@carbon/react'
import { Download, Report, View } from '@carbon/icons-react'
import { api, unwrap } from '../../api/client'
import type { SubmissionSummary } from '../../api/types'
import { EnvironmentSelect } from '../../components/EnvironmentSelect'
import { EmptyState, ErrorNotice, LoadingBlock, NeedsEnvironment } from '../../components/States'
import { controlNumber, downloadText, formatDateTime } from '../../lib/format'
import { useLoad } from '../../lib/useLoad'
import { useApp, useServerEvents } from '../../shell/AppContext'
import type { Feature } from '../../shell/feature'
import { toCsv, toJson } from './exportRows'
import { RESULT_TAG } from './resultTag'
import { SubmissionDetail } from './SubmissionDetail'

/** The newest rows the server returns; filters and exports work on these. */
const FETCH_LIMIT = 1000
const PAGE = 50

/** Local start of the day for a `YYYY-MM-DD` input value. */
function dayStart(value: string, endOfDay: boolean): number | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value)
  if (!m) return null
  const d = new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3]) + (endOfDay ? 1 : 0))
  return d.getTime()
}

function Activity() {
  const { selected } = useApp()
  const [domain, setDomain] = useState('')
  const [status, setStatus] = useState('')
  const [from, setFrom] = useState('')
  const [to, setTo] = useState('')
  const [shown, setShown] = useState(PAGE)
  const [openId, setOpenId] = useState<string | null>(null)

  const list = useLoad(
    selected
      ? () =>
          unwrap(
            api.GET('/environments/{id}/submissions', {
              params: { path: { id: selected.id }, query: { limit: FETCH_LIMIT } },
            }),
          )
      : null,
    [selected?.id],
  )
  useServerEvents((event) => {
    if (event.kind.startsWith('submission.')) void list.reload()
  })

  const all: SubmissionSummary[] = list.data ?? []
  const domains = useMemo(() => [...new Set(all.map((s) => s.domain))].sort(), [all])
  const rows = useMemo(() => {
    const start = dayStart(from, false)
    const end = dayStart(to, true)
    return all.filter((s) => {
      const t = new Date(s.submitted_at).getTime()
      return (
        (!domain || s.domain === domain) &&
        (!status || s.result === status) &&
        (start === null || t >= start) &&
        (end === null || t < end)
      )
    })
  }, [all, domain, status, from, to])

  const filtered = domain !== '' || status !== '' || from !== '' || to !== ''
  const reset = () => {
    setDomain('')
    setStatus('')
    setFrom('')
    setTo('')
    setShown(PAGE)
  }
  const stamp = new Date().toISOString().slice(0, 10)
  const name = `${selected?.name ?? 'activity'}-submissions-${stamp}`.replace(/[^\w.-]+/g, '_')

  return (
    <>
      <div className="operation-toolbar activity-environment">
        <EnvironmentSelect id="activity-environment" />
      </div>

      {!selected && <NeedsEnvironment />}
      {list.error && <ErrorNotice error={list.error} onRetry={() => void list.reload()} />}
      {selected && list.loading && !list.data && <LoadingBlock label="Loading activity" />}

      {selected && list.data && (
        <>
          <div className="activity-filters">
            <Select
              id="activity-domain"
              labelText="Business domain"
              value={domain}
              onChange={(e) => {
                setDomain(e.target.value)
                setShown(PAGE)
              }}
            >
              <SelectItem value="" text="All domains" />
              {domains.map((d) => (
                <SelectItem key={d} value={d} text={d} />
              ))}
            </Select>
            <Select
              id="activity-status"
              labelText="Result"
              value={status}
              onChange={(e) => {
                setStatus(e.target.value)
                setShown(PAGE)
              }}
            >
              <SelectItem value="" text="All results" />
              <SelectItem value="success" text="Submitted" />
              <SelectItem value="failed" text="Failed" />
              <SelectItem value="generated" text="Generated only" />
            </Select>
            <TextInput
              id="activity-from"
              type="date"
              labelText="From"
              value={from}
              onChange={(e) => {
                setFrom(e.target.value)
                setShown(PAGE)
              }}
            />
            <TextInput
              id="activity-to"
              type="date"
              labelText="To"
              value={to}
              onChange={(e) => {
                setTo(e.target.value)
                setShown(PAGE)
              }}
            />
            <Button kind="ghost" disabled={!filtered} onClick={reset}>
              Clear filters
            </Button>
          </div>

          <div className="section-title">
            <div>
              <h2>Submissions</h2>
              <p role="status" aria-live="polite">
                {rows.length} of {all.length} shown
                {all.length >= FETCH_LIMIT && ` (the ${FETCH_LIMIT} most recent)`}
              </p>
            </div>
            <div className="section-tags">
              <Button
                kind="tertiary"
                size="md"
                renderIcon={Download}
                disabled={rows.length === 0}
                onClick={() => downloadText(`${name}.csv`, toCsv(rows))}
              >
                Export CSV
              </Button>
              <Button
                kind="tertiary"
                size="md"
                renderIcon={Download}
                disabled={rows.length === 0}
                onClick={() => downloadText(`${name}.json`, toJson(rows))}
              >
                Export JSON
              </Button>
            </div>
          </div>

          {all.length === 0 ? (
            <EmptyState icon={<Report size={32} />} title="No submissions yet">
              Documents you submit from the Message studio are listed here with their control numbers and
              results.
            </EmptyState>
          ) : rows.length === 0 ? (
            <EmptyState icon={<Report size={32} />} title="Nothing matches these filters">
              Change or clear the filters to see more.
            </EmptyState>
          ) : (
            <div className="activity-table">
              <Table size="lg" useZebraStyles aria-label="Submissions">
                <TableHead>
                  <TableRow>
                    <TableHeader>Time</TableHeader>
                    <TableHeader>Document</TableHeader>
                    <TableHeader>Domain</TableHeader>
                    <TableHeader>Result</TableHeader>
                    <TableHeader>HTTP</TableHeader>
                    <TableHeader>ISA / GS</TableHeader>
                    <TableHeader>
                      <span className="visually-hidden">Actions</span>
                    </TableHeader>
                  </TableRow>
                </TableHead>
                <TableBody>
                  {rows.slice(0, shown).map((s) => {
                    const tag = RESULT_TAG[s.result]
                    return (
                      <TableRow key={s.id}>
                        <TableCell>{formatDateTime(s.submitted_at)}</TableCell>
                        <TableCell>
                          <strong>{s.transaction_code}</strong>
                        </TableCell>
                        <TableCell>{s.domain}</TableCell>
                        <TableCell>
                          <Tag type={tag.type}>{tag.text}</Tag>
                        </TableCell>
                        <TableCell>{s.http_status ?? '—'}</TableCell>
                        <TableCell>
                          {controlNumber(s.isa_control)} / {s.gs_control}
                        </TableCell>
                        <TableCell>
                          <Button
                            kind="ghost"
                            size="sm"
                            renderIcon={View}
                            aria-label={`View ${s.transaction_code} submission`}
                            onClick={() => setOpenId(s.id)}
                          >
                            View
                          </Button>
                        </TableCell>
                      </TableRow>
                    )
                  })}
                </TableBody>
              </Table>
              {rows.length > shown && (
                <Button kind="ghost" onClick={() => setShown((n) => n + PAGE)}>
                  Show {Math.min(PAGE, rows.length - shown)} more
                </Button>
              )}
            </div>
          )}
        </>
      )}
      {openId && <SubmissionDetail id={openId} onClose={() => setOpenId(null)} />}
    </>
  )
}

export const activityFeature: Feature = {
  id: 'activity',
  label: 'Activity',
  icon: Report,
  title: 'Activity',
  subtitle: 'Review what was submitted, with control numbers and results',
  Component: Activity,
}
