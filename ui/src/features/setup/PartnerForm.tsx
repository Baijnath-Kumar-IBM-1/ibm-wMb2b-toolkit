import { PasswordInput, Select, SelectItem, TextInput } from '@carbon/react'
import type { ApiFailure } from '../../api/client'
import type { PartnerDraft } from './partnerDraft'

export function PartnerForm({
  idPrefix,
  value,
  onChange,
  errors,
  disabled,
  passwordHelp,
}: {
  idPrefix: string
  value: PartnerDraft
  onChange: (next: PartnerDraft) => void
  errors?: ApiFailure | null
  disabled?: boolean
  passwordHelp?: string
}) {
  const set = (patch: Partial<PartnerDraft>) => onChange({ ...value, ...patch })
  const err = (field: string) => errors?.fieldMessage(field)
  const text = (field: keyof PartnerDraft, label: string, extra?: { helper?: string }) => (
    <TextInput
      id={`${idPrefix}-${field}`}
      labelText={label}
      helperText={extra?.helper}
      value={value[field]}
      disabled={disabled}
      invalid={Boolean(err(field))}
      invalidText={err(field)}
      onChange={(e) => set({ [field]: e.target.value })}
    />
  )
  return (
    <div className="form-grid">
      {text('name', 'Partner name')}
      {text('org_unit', 'Organization unit')}
      <div className="partner-id-group span-two">
        <Select id={`${idPrefix}-id-type`} labelText="ID type" value="duns" disabled>
          <SelectItem value="duns" text="DUNS" />
        </Select>
        <TextInput
          id={`${idPrefix}-id_value`}
          labelText="Partner ID (DUNS)"
          helperText="9 to 15 digits."
          inputMode="numeric"
          value={value.id_value}
          disabled={disabled}
          invalid={Boolean(err('id_value'))}
          invalidText={err('id_value')}
          onChange={(e) => set({ id_value: e.target.value })}
        />
      </div>
      <fieldset className="channel-credentials span-two">
        <legend>Channel invocation credentials</legend>
        <p>Used when documents are posted to this partner's channel.</p>
        <div>
          {text('username', 'Username')}
          <PasswordInput
            id={`${idPrefix}-password`}
            labelText="Password"
            autoComplete="new-password"
            helperText={passwordHelp ?? '8 to 128 characters. Stored in your system keychain.'}
            value={value.password}
            disabled={disabled}
            invalid={Boolean(err('password'))}
            invalidText={err('password')}
            onChange={(e) => set({ password: e.target.value })}
          />
          {text('full_name', 'User full name')}
          {text('channel_name', 'Channel name', {
            helper: 'Letters, digits and . _ - only.',
          })}
        </div>
      </fieldset>
    </div>
  )
}
