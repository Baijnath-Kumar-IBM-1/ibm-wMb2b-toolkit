import { Tag, Tile } from '@carbon/react'
import { UserMultiple } from '@carbon/icons-react'
import { api, unwrap } from '../../api/client'
import { EnvironmentSelect } from '../../components/EnvironmentSelect'
import { ErrorNotice, LoadingBlock, NeedsEnvironment } from '../../components/States'
import { useLoad } from '../../lib/useLoad'
import { useApp, useServerEvents } from '../../shell/AppContext'
import type { Feature } from '../../shell/feature'
import { NewPartnerPair } from './NewPartnerPair'
import { PartnerCard } from './PartnerCard'

function Setup() {
  const { selected } = useApp()
  const partners = useLoad(
    selected
      ? () => unwrap(api.GET('/environments/{id}/partners', { params: { path: { id: selected.id } } }))
      : null,
    [selected?.id],
  )
  useServerEvents((event) => {
    if (event.kind.startsWith('partner.')) void partners.reload()
  })

  const list = partners.data ?? []
  return (
    <>
      <div className="operation-toolbar">
        <EnvironmentSelect id="setup-environment" />
        {selected && <Tag type="green">{list.length} partners</Tag>}
      </div>
      {!selected && <NeedsEnvironment />}
      {selected && (
        <>
          <Tile className="partner-directory">
            <div className="directory-heading">
              <div>
                <h2>Partners in this environment</h2>
                <p>Provision each partner once; the Documents and Message studio screens use them.</p>
              </div>
              <Tag type="blue">{list.length} total</Tag>
            </div>
            {partners.loading && <LoadingBlock label="Loading partners" />}
            {partners.error && <ErrorNotice error={partners.error} onRetry={() => void partners.reload()} />}
            {partners.data && list.length === 0 && (
              <div className="empty-inline">
                <UserMultiple size={24} />
                <p>No partners yet. Create the first pair below.</p>
              </div>
            )}
            <div className="partner-directory-list">
              {list.map((p) => (
                <PartnerCard key={p.id} partner={p} onChanged={() => void partners.reload()} />
              ))}
            </div>
          </Tile>
          {partners.data && (
            <NewPartnerPair
              key={selected.id}
              environment={selected}
              existing={list}
              onCreated={() => void partners.reload()}
            />
          )}
        </>
      )}
    </>
  )
}

export const setupFeature: Feature = {
  id: 'setup',
  label: 'Setup',
  icon: UserMultiple,
  title: 'Setup',
  subtitle: 'Create reusable trading partners and communication channels',
  Component: Setup,
}
