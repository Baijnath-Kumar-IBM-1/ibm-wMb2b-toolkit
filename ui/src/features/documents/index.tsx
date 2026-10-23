import { useEffect, useMemo, useState } from 'react'
import { Button, Checkbox, InlineNotification, Select, SelectItem, Tag, Tile } from '@carbon/react'
import { Layers } from '@carbon/icons-react'
import { api, unwrap } from '../../api/client'
import type { InstallReport, Pack } from '../../api/types'
import { EnvironmentSelect } from '../../components/EnvironmentSelect'
import { RunProgress } from '../../components/RunProgress'
import { ErrorNotice, LoadingBlock, NeedsEnvironment } from '../../components/States'
import { useLoad } from '../../lib/useLoad'
import { useRun } from '../../lib/useRun'
import { useApp, useServerEvents } from '../../shell/AppContext'
import type { Feature } from '../../shell/feature'

const OUTCOME_TEXT: Record<InstallReport['documents'][number]['outcome'], string> = {
  installed: 'Installed',
  already_existed: 'Already on the tenant',
  already_installed: 'Already installed',
  failed: 'Failed',
  not_run: 'Not run',
}

function Documents() {
  const { selected, packs, notify } = useApp()
  const [packId, setPackId] = useState('')
  const [chosen, setChosen] = useState<string[]>([])

  const list = packs.data ?? []
  const pack: Pack | undefined = useMemo(
    () => list.find((p) => p.id === packId) ?? list[0],
    [list, packId],
  )

  const installed = useLoad(
    selected
      ? () =>
          unwrap(api.GET('/environments/{id}/documents', { params: { path: { id: selected.id } } }))
      : null,
    [selected?.id],
  )
  useServerEvents((event) => {
    if (event.kind.startsWith('document')) void installed.reload()
  })

  const handle = useRun((run) => {
    void installed.reload()
    if (run.status === 'failed') return
    const report = run.report as InstallReport | null | undefined
    const failed = report?.documents.filter((d) => d.outcome === 'failed').length ?? 0
    if (report?.cancelled) notify('Installation was cancelled', 'error')
    else if (failed) notify(`${failed} document${failed === 1 ? '' : 's'} failed to install`, 'error')
    else notify('Documents installed')
  })

  // Start from every document of the pack whenever the pack changes.
  useEffect(() => {
    if (pack) setChosen(pack.documents.map((d) => d.code))
  }, [pack?.id]) // eslint-disable-line react-hooks/exhaustive-deps

  const installedCodes = new Set((installed.data ?? []).map((d) => d.transaction_code))
  const all = pack ? chosen.length === pack.documents.length : false
  const toggle = (code: string) =>
    setChosen((c) => (c.includes(code) ? c.filter((x) => x !== code) : [...c, code]))
  const report = handle.run?.report as InstallReport | null | undefined

  return (
    <>
      <div className="operation-toolbar document-operation-toolbar">
        <EnvironmentSelect id="documents-environment" />
        <Select
          id="documents-pack"
          labelText="Business domain"
          value={pack?.id ?? ''}
          disabled={list.length === 0}
          onChange={(e) => setPackId(e.target.value)}
        >
          {list.map((p) => (
            <SelectItem key={p.id} value={p.id} text={p.label} />
          ))}
        </Select>
        <Button
          renderIcon={Layers}
          disabled={!selected || !pack || chosen.length === 0 || handle.running}
          onClick={() =>
            selected &&
            pack &&
            void handle.start(() =>
              unwrap(
                api.POST('/environments/{id}/documents/install', {
                  params: { path: { id: selected.id } },
                  body: { pack_id: pack.id, codes: chosen },
                }),
              ),
            )
          }
        >
          Install selected
        </Button>
      </div>

      {!selected && <NeedsEnvironment />}
      {packs.loading && <LoadingBlock label="Loading document packs" />}
      {packs.error && <ErrorNotice error={packs.error} onRetry={() => void packs.reload()} />}
      {installed.error && (
        <ErrorNotice
          title="Could not read installed documents"
          error={installed.error}
          onRetry={() => void installed.reload()}
        />
      )}
      <RunProgress handle={handle} label="Installing document types on the tenant" />
      {handle.error && (
        <InlineNotification
          className="wide-notification"
          kind="error"
          lowContrast
          hideCloseButton
          title="Could not start the installation"
          subtitle={handle.error.message}
        />
      )}
      {handle.run?.status === 'failed' && (
        <InlineNotification
          className="wide-notification"
          kind="error"
          lowContrast
          hideCloseButton
          title="Installation failed"
          subtitle={handle.run.error?.error ?? 'The run stopped unexpectedly.'}
        />
      )}

      {pack && (
        <>
          <div className="section-title">
            <div>
              <h2>Select your document profile</h2>
              <p>Choose a business domain, then pick the transaction sets to enable on this instance.</p>
            </div>
          </div>
          <Tile className="documents-tile">
            <div className="document-toolbar">
              <div>
                <h3>
                  {pack.label} transaction sets
                  <Tag type="cool-gray" className="inline-tag">
                    {pack.standard} {pack.version}
                  </Tag>
                </h3>
                <p>
                  {chosen.length} of {pack.documents.length} selected
                </p>
              </div>
              <Checkbox
                id="select-all-documents"
                labelText="Select all"
                checked={all}
                onChange={() => setChosen(all ? [] : pack.documents.map((d) => d.code))}
              />
            </div>
            <div className="document-list">
              {pack.documents.map((d) => {
                const result = report?.documents.find((r) => r.transaction_code === d.code)
                return (
                  <div className="document-row" key={d.code}>
                    <Checkbox
                      id={`document-${d.code}`}
                      labelText={`${d.code} ${d.name}`}
                      hideLabel
                      checked={chosen.includes(d.code)}
                      onChange={() => toggle(d.code)}
                    />
                    <strong>{d.code}</strong>
                    <div>
                      <h4>{d.name}</h4>
                      <p>{d.note}</p>
                      {result?.error && <p className="row-error">{result.error}</p>}
                    </div>
                    <div className="document-status">
                      {result && result.outcome !== 'not_run' && (
                        <Tag type={result.outcome === 'failed' ? 'red' : 'blue'}>
                          {OUTCOME_TEXT[result.outcome]}
                        </Tag>
                      )}
                      {installedCodes.has(d.code) && <Tag type="green">Installed</Tag>}
                    </div>
                  </div>
                )
              })}
            </div>
          </Tile>
        </>
      )}
    </>
  )
}

export const documentsFeature: Feature = {
  id: 'documents',
  label: 'Documents',
  icon: Layers,
  title: 'Document initialization',
  subtitle: 'Choose the transaction sets enabled for your domain',
  Component: Documents,
}
