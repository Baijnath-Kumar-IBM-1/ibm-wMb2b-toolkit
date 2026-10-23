import { useEffect, useMemo, useState } from 'react'
import { ActionableNotification, Button, Checkbox, InlineNotification, Select, SelectItem, TextInput, Tile } from '@carbon/react'
import { Send, Settings, UserMultiple, Document } from '@carbon/icons-react'
import { api, unwrap } from '../../api/client'
import type { Pack, SubmitReport } from '../../api/types'
import { EnvironmentSelect } from '../../components/EnvironmentSelect'
import { RunProgress } from '../../components/RunProgress'
import { ErrorNotice, NeedsEnvironment } from '../../components/States'
import { asRecord } from '../../lib/format'
import { useLoad } from '../../lib/useLoad'
import { useRun } from '../../lib/useRun'
import { useApp, useServerEvents } from '../../shell/AppContext'
import type { Feature } from '../../shell/feature'
import { MANUAL, PartnerPicker, toParty, type PartyChoice } from './PartnerPicker'
import { PreviewPane } from './PreviewPane'
import { SubmitResults, type Live } from './SubmitResults'
import { usePreview } from './usePreview'

function Studio() {
  const { selected, packs, notify, navigate } = useApp()
  const [packId, setPackId] = useState('')
  const [codes, setCodes] = useState<string[]>([])
  const [p1, setP1] = useState<PartyChoice>({ choice: MANUAL, manualId: '' })
  const [p2, setP2] = useState<PartyChoice>({ choice: MANUAL, manualId: '' })
  const [overrides, setOverrides] = useState<Record<string, string>>({})
  const [live, setLive] = useState<Record<string, Live>>({})

  const packList = packs.data ?? []
  const pack: Pack | undefined = useMemo(
    () => packList.find((p) => p.id === packId) ?? packList[0],
    [packList, packId],
  )

  const partners = useLoad(
    selected
      ? () => unwrap(api.GET('/environments/{id}/partners', { params: { path: { id: selected.id } } }))
      : null,
    [selected?.id],
  )
  const counters = useLoad(
    selected
      ? () => unwrap(api.GET('/environments/{id}/counters', { params: { path: { id: selected.id } } }))
      : null,
    [selected?.id],
  )
  const partnerList = partners.data ?? []

  // Preselect the first two saved partners when the environment changes.
  useEffect(() => {
    if (!partners.data) return
    setP1({ choice: partners.data[0]?.id ?? MANUAL, manualId: '' })
    setP2({ choice: partners.data[1]?.id ?? MANUAL, manualId: '' })
    setOverrides({})
  }, [partners.data])

  useEffect(() => {
    if (pack) setCodes(pack.documents.map((d) => d.code))
    setOverrides({})
  }, [pack?.id]) // eslint-disable-line react-hooks/exhaustive-deps

  const party1 = toParty(p1)
  const party2 = toParty(p2)
  const preview = usePreview({
    environmentId: selected?.id,
    packId: pack?.id,
    codes,
    p1: party1,
    p2: party2,
    overrides,
  })

  const handle = useRun((run) => {
    void counters.reload()
    if (run.status === 'failed') return notify('The submission could not complete', 'error')
    const report = run.report as SubmitReport | null | undefined
    const failed = report?.documents.filter((d) => d.outcome === 'failed').length ?? 0
    const sent = report?.documents.filter((d) => d.outcome === 'submitted').length ?? 0
    if (report?.cancelled) notify(`Cancelled after ${sent} document${sent === 1 ? '' : 's'}`, 'error')
    else if (failed) notify(`${failed} document${failed === 1 ? '' : 's'} failed`, 'error')
    else notify(`${sent} document${sent === 1 ? '' : 's'} submitted`)
  })

  useServerEvents((event) => {
    const data = asRecord(event.data)
    const code = typeof data.transaction_code === 'string' ? data.transaction_code : null
    if (!code) return
    if (event.kind === 'submission.posting') setLive((l) => ({ ...l, [code]: { state: 'posting' } }))
    if (event.kind === 'submission.result') {
      const ok = data.result === 'success'
      setLive((l) => ({
        ...l,
        [code]: {
          state: ok ? 'submitted' : 'failed',
          httpStatus: typeof data.http_status === 'number' ? data.http_status : undefined,
          error: typeof data.error === 'string' ? data.error : undefined,
        },
      }))
    }
  })

  const toggle = (code: string) =>
    setCodes((c) => (c.includes(code) ? c.filter((x) => x !== code) : [...c, code]))
  const ordered = pack ? pack.documents.filter((d) => codes.includes(d.code)).map((d) => d.code) : []
  const all = pack ? codes.length === pack.documents.length : false
  const savedBoth = p1.choice !== MANUAL && p2.choice !== MANUAL
  const sameSaved = savedBoth && p1.choice === p2.choice
  const traceKeys = Object.keys(preview.report?.trace_values ?? {})
  const finalReport = handle.run?.report as SubmitReport | null | undefined

  function submit() {
    if (!selected || !pack || !savedBoth) return
    setLive({})
    void handle.start(() =>
      unwrap(
        api.POST('/submit', {
          body: {
            environment_id: selected.id,
            pack_id: pack.id,
            codes: ordered,
            p1_partner_id: p1.choice,
            p2_partner_id: p2.choice,
            trace_overrides: overrides,
          },
        }),
      ),
    )
  }

  const submitHint = !savedBoth
    ? 'Pick two saved partners to submit.'
    : sameSaved
      ? 'Pick two different partners.'
      : ''

  return (
    <>
      <div className="operation-toolbar message-toolbar">
        <EnvironmentSelect id="message-environment" />
        <Select
          id="message-domain"
          labelText="Business domain"
          value={pack?.id ?? ''}
          disabled={packList.length === 0}
          onChange={(e) => setPackId(e.target.value)}
        >
          {packList.map((p) => (
            <SelectItem key={p.id} value={p.id} text={p.label} />
          ))}
        </Select>
        <Button
          renderIcon={Send}
          disabled={!selected || ordered.length === 0 || !savedBoth || sameSaved || handle.running}
          onClick={submit}
        >
          Submit {ordered.length > 1 ? `${ordered.length} documents` : 'document'}
        </Button>
      </div>

      {!selected && <NeedsEnvironment />}
      {packs.error && <ErrorNotice error={packs.error} onRetry={() => void packs.reload()} />}
      {partners.error && <ErrorNotice error={partners.error} onRetry={() => void partners.reload()} />}

      {selected && (
        <>
          {partners.data && partnerList.length < 2 && (
            <ActionableNotification
      hasFocus={false}
              inline
              className="wide-notification"
              kind="info"
              lowContrast
              hideCloseButton
              title="Submitting needs two provisioned partners"
              subtitle="You can still preview with partner IDs entered by hand."
              actionButtonLabel="Open Setup"
              onActionButtonClick={() => navigate('setup')}
            />
          )}
          <Tile className="partner-selection-tile">
            <div className="tile-heading">
              <UserMultiple size={20} />
              <div>
                <h2>Trading partner combination</h2>
                <p>Partner 1 and 2 follow the order shown in the document list.</p>
              </div>
            </div>
            <div className="partner-select-grid">
              <PartnerPicker id="message-partner-one" label="Partner 1" partners={partnerList} value={p1} onChange={setP1} />
              <PartnerPicker id="message-partner-two" label="Partner 2" partners={partnerList} value={p2} onChange={setP2} />
            </div>
          </Tile>

          <div className="studio-grid">
            <div className="studio-side">
              <Tile className="configuration-tile">
                <div className="tile-heading">
                  <Settings size={20} />
                  <div>
                    <h2>Document selection</h2>
                    <p>Select one or more documents to preview and submit.</p>
                  </div>
                </div>
                <div className="configuration-form">
                  <div className="document-multi-select">
                    <div className="document-multi-heading">
                      <span>Documents to submit</span>
                      <Checkbox
                        id="message-select-all"
                        labelText="All"
                        checked={all}
                        onChange={() => setCodes(all || !pack ? [] : pack.documents.map((d) => d.code))}
                      />
                    </div>
                    {pack?.documents.map((d) => (
                      <Checkbox
                        key={d.code}
                        id={`message-document-${d.code}`}
                        labelText={`${d.code} — ${d.name}`}
                        checked={codes.includes(d.code)}
                        onChange={() => toggle(d.code)}
                      />
                    ))}
                  </div>
                </div>
              </Tile>

              <Tile className="configuration-tile">
                <div className="tile-heading">
                  <Document size={20} />
                  <div>
                    <h2>Trace values</h2>
                    <p>These link the documents together. Edit to use your own.</p>
                  </div>
                </div>
                <div className="configuration-form">
                  {traceKeys.length === 0 && <p className="muted">Shown once a preview is available.</p>}
                  {traceKeys.map((key) => (
                    <TextInput
                      key={key}
                      id={`trace-${key}`}
                      labelText={key}
                      value={overrides[key] ?? preview.report?.trace_values[key] ?? ''}
                      helperText={key in overrides ? 'Custom value' : 'Generated'}
                      onChange={(e) => setOverrides((o) => ({ ...o, [key]: e.target.value }))}
                    />
                  ))}
                  {Object.keys(overrides).length > 0 && (
                    <Button kind="ghost" size="sm" onClick={() => setOverrides({})}>
                      Use generated values
                    </Button>
                  )}
                  {counters.data && (
                    <p className="muted">
                      Next control numbers: ISA {counters.data.isa_next}, GS {counters.data.gs_next} · {counters.data.run_count} earlier runs
                    </p>
                  )}
                </div>
              </Tile>
            </div>

            <Tile className="preview-tile">
              <PreviewPane
                report={preview.report}
                error={preview.error}
                loading={preview.loading}
                onRetry={preview.refresh}
                idle={
                  ordered.length === 0
                    ? 'Select at least one document.'
                    : 'Choose both partners (or enter valid DUNS numbers) to see the EDI.'
                }
              />
              {preview.report && (
                <p className="preview-note muted">
                  Preview only: nothing is sent and no control numbers are used until you submit.
                </p>
              )}
            </Tile>
          </div>

          {submitHint && <p className="muted submit-hint">{submitHint}</p>}
          <RunProgress handle={handle} label="Submitting documents" />
          {handle.error && (
            <InlineNotification
              className="wide-notification"
              kind="error"
              lowContrast
              hideCloseButton
              title="Could not start the submission"
              subtitle={handle.error.message}
            />
          )}
          {handle.run?.status === 'failed' && (
            <InlineNotification
              className="wide-notification"
              kind="error"
              lowContrast
              hideCloseButton
              title="Submission failed"
              subtitle={handle.run.error?.error ?? 'The run stopped unexpectedly.'}
            />
          )}
          {(handle.running || handle.run) && (
            <SubmitResults codes={ordered} live={live} report={finalReport} />
          )}
        </>
      )}
    </>
  )
}

export const studioFeature: Feature = {
  id: 'studio',
  label: 'Message studio',
  icon: Document,
  title: 'EDI message studio',
  subtitle: 'Preview linked EDI documents and submit them to your B2B instance',
  Component: Studio,
}
