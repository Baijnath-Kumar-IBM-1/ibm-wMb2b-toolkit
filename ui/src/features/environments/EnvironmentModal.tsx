import { useState } from 'react'
import {
  Accordion,
  AccordionItem,
  Button,
  InlineLoading,
  InlineNotification,
  Modal,
  PasswordInput,
  Tag,
  TextInput,
} from '@carbon/react'
import { Edit, Star, TrashCan } from '@carbon/icons-react'
import { api, ApiFailure, unwrap } from '../../api/client'
import type { Environment } from '../../api/types'
import { useApp } from '../../shell/AppContext'

const IAM_HOST_TEST = 'account-iam.platform.test.saas.ibm.com'
const IAM_HOST_PROD = 'account-iam.platform.saas.ibm.com'

interface Draft {
  id: string
  name: string
  b2b_host: string
  instance_id: string
  api_key: string
  iam_host: string
  ca_bundle_path: string
}

const EMPTY: Draft = {
  id: '',
  name: '',
  b2b_host: '',
  instance_id: '',
  api_key: '',
  iam_host: IAM_HOST_TEST,
  ca_bundle_path: '',
}

function toDraft(e: Environment): Draft {
  return {
    id: e.id,
    name: e.name,
    b2b_host: e.b2b_host,
    instance_id: e.instance_id,
    api_key: '',
    iam_host: e.iam_host,
    ca_bundle_path: e.ca_bundle_path ?? '',
  }
}

export function EnvironmentModal() {
  const { settingsOpen, closeSettings, environments, selectEnvironment, notify } = useApp()
  const [draft, setDraft] = useState<Draft>(EMPTY)
  const [saving, setSaving] = useState(false)
  const [failure, setFailure] = useState<ApiFailure | null>(null)
  const [testing, setTesting] = useState(false)
  const [test, setTest] = useState<{ ok: boolean; message: string } | null>(null)
  const [confirmDelete, setConfirmDelete] = useState('')

  const list = environments.data ?? []
  const editing = list.find((e) => e.id === draft.id)
  const set = (patch: Partial<Draft>) => setDraft((d) => ({ ...d, ...patch }))
  const fieldError = (field: string) => failure?.fieldMessage(field)

  const complete =
    draft.name.trim() !== '' &&
    draft.b2b_host.trim() !== '' &&
    draft.instance_id.trim() !== '' &&
    (editing !== undefined || draft.api_key !== '')

  function begin(next: Draft) {
    setDraft(next)
    setFailure(null)
    setTest(null)
    setConfirmDelete('')
  }

  function close() {
    begin(EMPTY)
    closeSettings()
  }

  async function save() {
    setSaving(true)
    setFailure(null)
    const body = {
      name: draft.name.trim(),
      b2b_host: draft.b2b_host.trim(),
      instance_id: draft.instance_id.trim(),
      iam_host: draft.iam_host.trim() || IAM_HOST_TEST,
      ca_bundle_path: draft.ca_bundle_path.trim() || null,
      ...(draft.api_key ? { api_key: draft.api_key } : {}),
    }
    try {
      const saved = editing
        ? await unwrap(
            api.PUT('/environments/{id}', { params: { path: { id: editing.id } }, body }),
          )
        : await unwrap(api.POST('/environments', { body: { ...body, api_key: draft.api_key } }))
      await environments.reload()
      selectEnvironment(saved.id)
      // Stay open in edit mode so the connection can be tested right away.
      begin(toDraft(saved))
      notify(editing ? 'Environment updated' : 'Environment added')
    } catch (e) {
      setFailure(e instanceof ApiFailure ? e : new ApiFailure(0, undefined))
    } finally {
      setSaving(false)
    }
  }

  async function testConnection() {
    if (!editing) return
    setTesting(true)
    setTest(null)
    try {
      setTest(
        await unwrap(
          api.POST('/environments/{id}/test', { params: { path: { id: editing.id } } }),
        ),
      )
    } catch (e) {
      setTest({ ok: false, message: e instanceof Error ? e.message : 'Test failed' })
    } finally {
      setTesting(false)
    }
  }

  async function makeDefault(id: string) {
    try {
      await unwrap(api.POST('/environments/{id}/default', { params: { path: { id } } }))
      await environments.reload()
      selectEnvironment(id)
      notify('Default environment updated')
    } catch (e) {
      notify(e instanceof Error ? e.message : 'Could not update the default', 'error')
    }
  }

  async function remove(id: string) {
    try {
      await unwrap(api.DELETE('/environments/{id}', { params: { path: { id } } }))
      await environments.reload()
      if (draft.id === id) begin(EMPTY)
      notify('Environment deleted')
    } catch (e) {
      notify(e instanceof Error ? e.message : 'Could not delete the environment', 'error')
    } finally {
      setConfirmDelete('')
    }
  }

  return (
    <Modal
      open={settingsOpen}
      modalHeading="Environment settings"
      modalLabel="webMethods B2B connection"
      primaryButtonText={editing ? 'Update environment' : 'Add environment'}
      secondaryButtonText="Close"
      primaryButtonDisabled={!complete || saving}
      onRequestClose={close}
      onSecondarySubmit={close}
      onRequestSubmit={() => void save()}
      hasScrollingContent
      aria-label="Environment settings"
    >
      {list.length > 0 && (
        <div className="environment-list" role="list">
          {list.map((environment) => (
            <div className="environment-row" key={environment.id} role="listitem">
              <div>
                <strong>{environment.name}</strong>
                <span>{environment.b2b_host}</span>
              </div>
              {environment.is_default ? (
                <Tag type="green">Default</Tag>
              ) : (
                <Button
                  kind="ghost"
                  size="sm"
                  renderIcon={Star}
                  onClick={() => void makeDefault(environment.id)}
                >
                  Set default
                </Button>
              )}
              <div className="row-actions">
                <Button
                  hasIconOnly
                  kind="ghost"
                  size="sm"
                  renderIcon={Edit}
                  iconDescription={`Edit ${environment.name}`}
                  onClick={() => begin(toDraft(environment))}
                />
                {confirmDelete === environment.id ? (
                  <Button
                    kind="danger--ghost"
                    size="sm"
                    onClick={() => void remove(environment.id)}
                  >
                    Confirm delete
                  </Button>
                ) : (
                  <Button
                    hasIconOnly
                    kind="ghost"
                    size="sm"
                    renderIcon={TrashCan}
                    iconDescription={`Delete ${environment.name}`}
                    onClick={() => setConfirmDelete(environment.id)}
                  />
                )}
              </div>
            </div>
          ))}
        </div>
      )}
      {confirmDelete && (
        <InlineNotification
          className="wide-notification"
          kind="warning"
          lowContrast
          hideCloseButton
          title="Deleting an environment removes its saved partners, history and secrets from this computer."
          subtitle="Nothing is removed from the B2B tenant."
        />
      )}

      <Accordion className="help-accordion">
        <AccordionItem title="Where to find these values" open={list.length === 0}>
          <ol className="help-steps">
            <li>
              <strong>Instance ID.</strong> Sign in to the IBM SaaS Console, choose your SaaS account,
              open the <em>IBM webMethods Hybrid Integration</em> subscription, then{' '}
              <em>View instances</em> and pick the instance. Copy the Instance ID from the
              <em> Instance details</em> tab (a UUID).
            </li>
            <li>
              <strong>API key.</strong> On the instance page open <em>Access</em> (or <em>API keys</em>),
              choose <em>Generate API key</em>, scope it to this instance, and copy it straight away
              as it is shown once. Treat it like a password.
            </li>
            <li>
              <strong>B2B host.</strong> Open the instance, then <em>webMethods B2B Integration</em>.
              The host name in the browser address bar (between <code>https://</code> and the first{' '}
              <code>/</code>) is the B2B host, for example
              <code> dev2205134-m-yqb-c1.b2b.ipaas.ibm.com</code>. Enter it without{' '}
              <code>https://</code>.
            </li>
          </ol>
        </AccordionItem>
      </Accordion>

      <div className="settings-form">
        {failure && !failure.fields.length && (
          <InlineNotification
            className="wide-notification"
            kind="error"
            lowContrast
            hideCloseButton
            title="Could not save"
            subtitle={failure.message}
          />
        )}
        <TextInput
          id="environment-name"
          labelText="Environment name"
          helperText="A recognizable name such as Production, QA or Sandbox."
          placeholder="Sandbox"
          value={draft.name}
          invalid={Boolean(fieldError('name'))}
          invalidText={fieldError('name')}
          onChange={(e) => set({ name: e.target.value })}
        />
        <TextInput
          id="environment-host"
          labelText="B2B host"
          helperText="Host name only, without https://."
          placeholder="your-instance.b2b.ipaas.ibm.com"
          value={draft.b2b_host}
          invalid={Boolean(fieldError('b2b_host'))}
          invalidText={fieldError('b2b_host')}
          onChange={(e) => set({ b2b_host: e.target.value })}
        />
        <TextInput
          id="environment-instance"
          labelText="Instance ID"
          helperText="The UUID from the instance details page."
          placeholder="xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx"
          value={draft.instance_id}
          invalid={Boolean(fieldError('instance_id'))}
          invalidText={fieldError('instance_id')}
          onChange={(e) => set({ instance_id: e.target.value })}
        />
        <PasswordInput
          id="environment-api-key"
          labelText="API key"
          autoComplete="off"
          helperText={
            editing?.has_api_key
              ? 'A key is stored in your system keychain. Leave empty to keep it, or paste a new one.'
              : 'Stored in your system keychain and never shown again.'
          }
          value={draft.api_key}
          invalid={Boolean(fieldError('api_key'))}
          invalidText={fieldError('api_key')}
          onChange={(e) => set({ api_key: e.target.value })}
        />

        <Accordion>
          <AccordionItem title="Advanced">
            <div className="settings-form">
              <TextInput
                id="environment-iam"
                labelText="IAM host"
                helperText={`${IAM_HOST_TEST} (test, default) or ${IAM_HOST_PROD} (production).`}
                value={draft.iam_host}
                invalid={Boolean(fieldError('iam_host'))}
                invalidText={fieldError('iam_host')}
                onChange={(e) => set({ iam_host: e.target.value })}
              />
              <TextInput
                id="environment-ca"
                labelText="CA certificate bundle (optional)"
                helperText="Absolute path to a PEM file, for a network that re-signs HTTPS traffic. Certificate checks are never turned off."
                placeholder="/etc/ssl/company-ca.pem"
                value={draft.ca_bundle_path}
                invalid={Boolean(fieldError('ca_bundle_path'))}
                invalidText={fieldError('ca_bundle_path')}
                onChange={(e) => set({ ca_bundle_path: e.target.value })}
              />
            </div>
          </AccordionItem>
        </Accordion>

        <div className="test-connection">
          <Button
            kind="tertiary"
            size="md"
            disabled={!editing || testing || saving}
            onClick={() => void testConnection()}
          >
            Test connection
          </Button>
          {testing && <InlineLoading description="Contacting the tenant" />}
          {!editing && <span className="muted">Save the environment to test it.</span>}
        </div>
        {test && (
          <InlineNotification
            className="wide-notification"
            kind={test.ok ? 'success' : 'error'}
            lowContrast
            hideCloseButton
            title={test.ok ? 'Connected' : 'Connection failed'}
            subtitle={test.message}
          />
        )}
        {editing && (
          <Button kind="ghost" size="sm" onClick={() => begin(EMPTY)}>
            Add another environment
          </Button>
        )}
      </div>
    </Modal>
  )
}
