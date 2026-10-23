import { useState } from 'react'
import { Button, InlineNotification, Modal, Tag } from '@carbon/react'
import { Edit, Renew, TrashCan } from '@carbon/icons-react'
import { api, ApiFailure, unwrap } from '../../api/client'
import type { Partner, Provisioning, StepRunReport } from '../../api/types'
import { StatusTag } from '../../components/StatusTag'
import { RunProgress } from '../../components/RunProgress'
import { useRun } from '../../lib/useRun'
import { useApp, useServerEvents } from '../../shell/AppContext'
import { useLoad } from '../../lib/useLoad'
import { PartnerForm } from './PartnerForm'
import {
  bodyFromDraft,
  draftFromPartner,
  PROVISION_STEP_LABELS,
  type PartnerDraft,
} from './partnerDraft'

export function PartnerCard({ partner, onChanged }: { partner: Partner; onChanged: () => void }) {
  const { notify } = useApp()
  const provisioning = useLoad(
    () => unwrap(api.GET('/partners/{id}/provisioning', { params: { path: { id: partner.id } } })),
    [partner.id],
  )
  const [editing, setEditing] = useState<PartnerDraft | null>(null)
  const [editError, setEditError] = useState<ApiFailure | null>(null)
  const [confirmDelete, setConfirmDelete] = useState(false)

  const handle = useRun((run) => {
    void provisioning.reload()
    if (run.status === 'failed') return
    const report = run.report as StepRunReport | null | undefined
    if (report?.cancelled) notify(`Provisioning of ${partner.name} was cancelled`, 'error')
    else if (report?.steps.some((s) => s.outcome === 'failed')) {
      notify(`Provisioning of ${partner.name} stopped at a failed step`, 'error')
    } else notify(`${partner.name} is provisioned`)
  })

  useServerEvents((event) => {
    if (event.kind.startsWith('partner.provision')) void provisioning.reload()
  })

  const state: Provisioning | undefined = provisioning.data
  const anySucceeded = state?.steps.some((s) => s.status === 'succeeded') ?? false
  const failedStep = state?.steps.find((s) => s.status === 'failed')

  async function saveEdit() {
    if (!editing) return
    setEditError(null)
    try {
      await unwrap(
        api.PUT('/partners/{id}', {
          params: { path: { id: partner.id } },
          body: bodyFromDraft(editing),
        }),
      )
      setEditing(null)
      onChanged()
      notify('Partner updated')
    } catch (e) {
      setEditError(e instanceof ApiFailure ? e : new ApiFailure(0, undefined))
    }
  }

  async function remove() {
    try {
      await unwrap(api.DELETE('/partners/{id}', { params: { path: { id: partner.id } } }))
      notify(`${partner.name} removed from this computer`)
      onChanged()
    } catch (e) {
      notify(e instanceof Error ? e.message : 'Could not delete the partner', 'error')
    }
  }

  return (
    <div className="partner-card">
      <div className="saved-partner">
        <div className="saved-partner-avatar" aria-hidden="true">
          {partner.name.slice(0, 2).toUpperCase()}
        </div>
        <div>
          <strong>{partner.name}</strong>
          <span>
            DUNS · {partner.id_value}
          </span>
        </div>
        <Tag type="cool-gray">{partner.channel_name}</Tag>
      </div>

      <div className="provision-steps">
        {provisioning.loading && <span className="muted">Loading status</span>}
        {provisioning.error && <span className="muted">Status unavailable</span>}
        {state && (
          <ol aria-label={`Provisioning steps for ${partner.name}`}>
            {state.steps.map((s) => (
              <li key={s.name}>
                <span>{PROVISION_STEP_LABELS[s.name] ?? s.name}</span>
                <StatusTag kind={s.status} />
              </li>
            ))}
          </ol>
        )}
      </div>

      {failedStep?.error && !handle.running && (
        <InlineNotification
          className="wide-notification"
          kind="error"
          lowContrast
          hideCloseButton
          title={`${PROVISION_STEP_LABELS[failedStep.name] ?? failedStep.name} failed`}
          subtitle={failedStep.error}
        />
      )}
      {handle.error && (
        <InlineNotification
          className="wide-notification"
          kind="error"
          lowContrast
          hideCloseButton
          title="Could not start provisioning"
          subtitle={handle.error.message}
        />
      )}
      <RunProgress handle={handle} label="Provisioning on the tenant" />

      <div className="partner-actions">
        {state && !state.complete && (
          <Button
            size="sm"
            kind={failedStep ? 'primary' : 'tertiary'}
            renderIcon={Renew}
            disabled={handle.running}
            onClick={() =>
              void handle.start(() =>
                unwrap(
                  api.POST('/partners/{id}/provision', { params: { path: { id: partner.id } } }),
                ),
              )
            }
          >
            {failedStep ? 'Retry' : anySucceeded ? 'Resume' : 'Provision'}
          </Button>
        )}
        {state?.complete && <Tag type="green">Ready for documents</Tag>}
        <Button
          hasIconOnly
          kind="ghost"
          size="sm"
          renderIcon={Edit}
          iconDescription={
            anySucceeded ? 'Cannot edit after provisioning has started' : `Edit ${partner.name}`
          }
          disabled={anySucceeded || handle.running}
          onClick={() => {
            setEditError(null)
            setEditing(draftFromPartner(partner))
          }}
        />
        {confirmDelete ? (
          <>
            <Button kind="danger--ghost" size="sm" onClick={() => void remove()}>
              Confirm delete
            </Button>
            <Button kind="ghost" size="sm" onClick={() => setConfirmDelete(false)}>
              Keep
            </Button>
          </>
        ) : (
          <Button
            hasIconOnly
            kind="ghost"
            size="sm"
            renderIcon={TrashCan}
            iconDescription={`Delete ${partner.name}`}
            disabled={handle.running}
            onClick={() => setConfirmDelete(true)}
          />
        )}
      </div>
      {confirmDelete && (
        <p className="muted partner-note">
          This removes the partner from this computer only. Anything already created on the tenant stays there.
        </p>
      )}

      <Modal
        open={editing !== null}
        modalHeading={`Edit ${partner.name}`}
        modalLabel="Trading partner"
        primaryButtonText="Save changes"
        secondaryButtonText="Cancel"
        onRequestClose={() => setEditing(null)}
        onRequestSubmit={() => void saveEdit()}
        hasScrollingContent
        aria-label={`Edit ${partner.name}`}
      >
        {editing && (
          <>
            {editError && !editError.fields.length && (
              <InlineNotification
                className="wide-notification"
                kind="error"
                lowContrast
                hideCloseButton
                title="Could not save"
                subtitle={editError.message}
              />
            )}
            <PartnerForm
              idPrefix={`edit-${partner.id}`}
              value={editing}
              onChange={setEditing}
              errors={editError}
              passwordHelp="Leave empty to keep the stored password."
            />
          </>
        )}
      </Modal>
    </div>
  )
}
