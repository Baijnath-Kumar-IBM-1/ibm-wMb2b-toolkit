import { ActionableNotification, Button, Tile } from '@carbon/react'
import { ArrowRight, Dashboard, Document, Layers, UserMultiple } from '@carbon/icons-react'
import { api, unwrap } from '../../api/client'
import type { SubmissionSummary } from '../../api/types'
import { ErrorNotice, LoadingBlock } from '../../components/States'
import { useLoad } from '../../lib/useLoad'
import { useApp, useServerEvents } from '../../shell/AppContext'
import type { Feature } from '../../shell/feature'

function Overview() {
  const { selected, openSettings, navigate } = useApp()
  const load = <T,>(fn: (id: string) => Promise<T>) => (selected ? () => fn(selected.id) : null)
  const partners = useLoad(
    load((id) => unwrap(api.GET('/environments/{id}/partners', { params: { path: { id } } }))),
    [selected?.id],
  )
  const documents = useLoad(
    load((id) => unwrap(api.GET('/environments/{id}/documents', { params: { path: { id } } }))),
    [selected?.id],
  )
  const submissions = useLoad(
    load((id) =>
      unwrap(
        api.GET('/environments/{id}/submissions', {
          params: { path: { id }, query: { limit: 1000 } },
        }),
      ),
    ),
    [selected?.id],
  )
  useServerEvents((event) => {
    if (event.kind.startsWith('partner.')) void partners.reload()
    if (event.kind.startsWith('document')) void documents.reload()
    if (event.kind.startsWith('submission.')) void submissions.reload()
  })

  const sent: SubmissionSummary[] = submissions.data ?? []
  const succeeded = sent.filter((s) => s.result === 'success').length
  const failed = sent.filter((s) => s.result === 'failed').length
  const error = partners.error ?? documents.error ?? submissions.error
  const waiting = selected && (partners.loading || documents.loading || submissions.loading)

  const metrics: [string, string][] = [
    ['Environment', selected?.name ?? 'Not configured'],
    ['Partners', String(partners.data?.length ?? 0)],
    ['Document types installed', String(documents.data?.length ?? 0)],
    ['Documents submitted', `${succeeded}${failed ? ` (${failed} failed)` : ''}`],
  ]
  const quickLinks: [string, string, string, typeof UserMultiple][] = [
    ['Set up partners', 'Create and provision both trading partners.', 'setup', UserMultiple],
    ['Choose documents', 'Install the transaction sets for your business domain.', 'documents', Layers],
    ['Generate a message', 'Preview linked EDI documents and submit them.', 'studio', Document],
  ]

  return (
    <>
      {!selected && (
        <ActionableNotification
      hasFocus={false}
          inline
          className="overview-notification"
          kind="info"
          lowContrast
          hideCloseButton
          title="Connect your B2B environment"
          subtitle="Add the B2B host, instance ID and API key to start."
          actionButtonLabel="Configure"
          onActionButtonClick={openSettings}
        />
      )}
      {error && (
        <ErrorNotice
          error={error}
          onRetry={() => {
            void partners.reload()
            void documents.reload()
            void submissions.reload()
          }}
        />
      )}
      {waiting && !error ? (
        <LoadingBlock label="Loading overview" />
      ) : (
        <div className="metric-grid">
          {metrics.map(([label, value]) => (
            <Tile className="metric-tile" key={label}>
              <p>{label}</p>
              <strong>{value}</strong>
            </Tile>
          ))}
        </div>
      )}
      <div className="section-title">
        <div>
          <h2>Continue setup</h2>
          <p>Complete each step to begin exchanging EDI messages.</p>
        </div>
      </div>
      <div className="quick-grid">
        {quickLinks.map(([title, text, target, Icon], index) => (
          <Tile className="quick-tile" key={title}>
            <div className="quick-number">0{index + 1}</div>
            <Icon size={24} />
            <h3>{title}</h3>
            <p>{text}</p>
            <Button kind="ghost" size="sm" renderIcon={ArrowRight} onClick={() => navigate(target)}>
              Open step
            </Button>
          </Tile>
        ))}
      </div>
    </>
  )
}

export const overviewFeature: Feature = {
  id: 'overview',
  label: 'Overview',
  icon: Dashboard,
  title: 'Overview',
  subtitle: 'Monitor your webMethods B2B workspace',
  Component: Overview,
}
