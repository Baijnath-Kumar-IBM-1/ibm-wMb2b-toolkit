import { useState } from 'react'
import { Button, InlineNotification, Tag, Tile } from '@carbon/react'
import { DataBase, Information, UserMultiple } from '@carbon/icons-react'
import { api, ApiFailure, unwrap } from '../../api/client'
import type { Environment, Partner } from '../../api/types'
import { ActionBar } from '../../components/ActionBar'
import { useApp } from '../../shell/AppContext'
import { PartnerForm } from './PartnerForm'
import { bodyFromDraft, samplePair, type PartnerDraft } from './partnerDraft'

const ROLES = [
  { title: 'Partner 1', text: 'Buyer / initiating organization', tag: 'Initiator', color: 'purple' },
  { title: 'Partner 2', text: 'Supplier / receiving organization', tag: 'Receiver', color: 'cyan' },
] as const

export function NewPartnerPair({
  environment,
  existing,
  onCreated,
}: {
  environment: Environment
  existing: Partner[]
  onCreated: () => void
}) {
  const { notify } = useApp()
  const [pair, setPair] = useState<[PartnerDraft, PartnerDraft]>(() => samplePair(existing))
  const [created, setCreated] = useState<[boolean, boolean]>([false, false])
  const [errors, setErrors] = useState<[ApiFailure | null, ApiFailure | null]>([null, null])
  const [busy, setBusy] = useState(false)

  const update = (index: 0 | 1, next: PartnerDraft) =>
    setPair((p) => (index === 0 ? [next, p[1]] : [p[0], next]))

  async function create() {
    setBusy(true)
    const nextErrors: [ApiFailure | null, ApiFailure | null] = [null, null]
    const nextCreated: [boolean, boolean] = [...created]
    // Created one after the other; a partner that already went through is not sent twice.
    for (const index of [0, 1] as const) {
      if (nextCreated[index]) continue
      try {
        await unwrap(
          api.POST('/environments/{id}/partners', {
            params: { path: { id: environment.id } },
            body: { ...bodyFromDraft(pair[index]), password: pair[index].password },
          }),
        )
        nextCreated[index] = true
      } catch (e) {
        nextErrors[index] = e instanceof ApiFailure ? e : new ApiFailure(0, undefined)
      }
    }
    setCreated(nextCreated)
    setErrors(nextErrors)
    setBusy(false)
    if (nextCreated[0] || nextCreated[1]) onCreated()
    if (nextCreated[0] && nextCreated[1]) {
      notify(`${pair[0].name} and ${pair[1].name} added. Provision them from the list above.`)
      setPair(samplePair([...existing, ...pair.map((d) => ({ name: d.name }) as Partner)]))
      setCreated([false, false])
    }
  }

  const complete = pair.every((d) => d.name && d.id_value && d.password && d.username && d.channel_name)

  return (
    <>
      <div className="section-title">
        <div>
          <h2>Trading partner details</h2>
          <p>Sample data is filled in so you can review the structure before creating.</p>
        </div>
        <Tag type="purple" renderIcon={Information}>
          Sample data loaded
        </Tag>
      </div>
      <div className="partner-grid">
        {ROLES.map((role, i) => {
          const index = i as 0 | 1
          return (
            <Tile className="partner-tile" key={role.title}>
              <div className="tile-heading">
                <div className="partner-symbol" aria-hidden="true">
                  P{index + 1}
                </div>
                <div>
                  <h3>{role.title}</h3>
                  <p>{role.text}</p>
                </div>
                {created[index] ? <Tag type="green">Created</Tag> : <Tag type={role.color}>{role.tag}</Tag>}
              </div>
              {errors[index] && !errors[index]?.fields.length && (
                <InlineNotification
                  className="wide-notification"
                  kind="error"
                  lowContrast
                  hideCloseButton
                  title={`${role.title} was not created`}
                  subtitle={errors[index]?.message}
                />
              )}
              <PartnerForm
                idPrefix={`new-p${index + 1}`}
                value={pair[index]}
                onChange={(next) => update(index, next)}
                errors={errors[index]}
                disabled={created[index] || busy}
              />
            </Tile>
          )
        })}
      </div>
      <ActionBar
        icon={<DataBase size={20} />}
        title="Ready to save this setup"
        detail="Both partners are saved here first, then provisioned on the tenant"
      >
        <Button
          kind="secondary"
          size="md"
          disabled={busy}
          onClick={() => {
            setPair(samplePair(existing))
            setCreated([false, false])
            setErrors([null, null])
          }}
        >
          New sample data
        </Button>
        <Button size="md" renderIcon={UserMultiple} disabled={busy || !complete} onClick={() => void create()}>
          {busy ? 'Creating' : 'Create partners'}
        </Button>
      </ActionBar>
    </>
  )
}
