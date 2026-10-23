import { Select, SelectItem, TextInput } from '@carbon/react'
import type { Partner, Party } from '../../api/types'

export const MANUAL = 'manual'

export interface PartyChoice {
  /** A partner id, or `MANUAL`. */
  choice: string
  manualId: string
}

export function toParty(c: PartyChoice): Party | null {
  if (c.choice === MANUAL) {
    return /^\d{9,15}$/.test(c.manualId) ? { type: 'manual', id_value: c.manualId } : null
  }
  return c.choice ? { type: 'saved', partner_id: c.choice } : null
}

export function PartnerPicker({
  id,
  label,
  partners,
  value,
  onChange,
}: {
  id: string
  label: string
  partners: Partner[]
  value: PartyChoice
  onChange: (next: PartyChoice) => void
}) {
  const manualInvalid = value.choice === MANUAL && value.manualId !== '' && toParty(value) === null
  return (
    <div className="partner-picker">
      <Select
        id={id}
        labelText={label}
        value={value.choice}
        onChange={(e) => onChange({ ...value, choice: e.target.value })}
      >
        <SelectItem value={MANUAL} text="Enter partner ID manually" />
        {partners.map((p) => (
          <SelectItem key={p.id} value={p.id} text={`${p.name} (${p.id_value})`} />
        ))}
      </Select>
      {value.choice === MANUAL && (
        <TextInput
          id={`${id}-manual`}
          labelText={`${label} ID (DUNS)`}
          helperText="9 to 15 digits. Preview only; submitting needs a saved partner."
          inputMode="numeric"
          placeholder="123456789"
          value={value.manualId}
          invalid={manualInvalid}
          invalidText="Enter 9 to 15 digits"
          onChange={(e) => onChange({ ...value, manualId: e.target.value.trim() })}
        />
      )}
    </div>
  )
}
