import type { Partner } from '../../api/types'
import { randomDuns, randomHex, randomPassword } from '../../lib/format'

export interface PartnerDraft {
  name: string
  org_unit: string
  id_value: string
  username: string
  password: string
  full_name: string
  channel_name: string
}

export const PROVISION_STEP_LABELS: Record<string, string> = {
  create_partner: 'Create partner',
  create_channel: 'Create channel',
  associate_channel: 'Link channel',
  create_user: 'Create user',
  associate_user: 'Link user',
}

/**
 * Two fresh sample partners (a buyer and a supplier). Names, user names and
 * channel names are unique per environment, so a short random suffix is
 * added when the plain name is already taken.
 */
export function samplePair(existing: Partner[]): [PartnerDraft, PartnerDraft] {
  const taken = new Set(existing.map((p) => p.name.toLowerCase()))
  const unique = (name: string) => (taken.has(name.toLowerCase()) ? `${name} ${randomHex(2)}` : name)
  const tag = randomHex(2)
  return [
    {
      name: unique('Acme Retail Group'),
      org_unit: 'Buyer Operations',
      id_value: randomDuns(),
      username: `acme.admin.${tag}`,
      password: randomPassword(),
      full_name: 'Ravi Kumar',
      channel_name: `ACME_OUTBOUND_${tag}`,
    },
    {
      name: unique('Preferred Supplier Ltd'),
      org_unit: 'Order Fulfillment',
      id_value: randomDuns(),
      username: `supplier.admin.${tag}`,
      password: randomPassword(),
      full_name: 'Meera Shah',
      channel_name: `SUPPLIER_INBOUND_${tag}`,
    },
  ]
}

export function draftFromPartner(p: Partner): PartnerDraft {
  return {
    name: p.name,
    org_unit: p.org_unit,
    id_value: p.id_value,
    username: p.username,
    password: '',
    full_name: p.full_name,
    channel_name: p.channel_name,
  }
}

/** Request body; an empty password is left out so an update keeps the stored one. */
export function bodyFromDraft(d: PartnerDraft) {
  return {
    name: d.name.trim(),
    org_unit: d.org_unit.trim(),
    id_type: 'duns' as const,
    id_value: d.id_value.trim(),
    username: d.username.trim(),
    full_name: d.full_name.trim(),
    channel_name: d.channel_name.trim(),
    ...(d.password ? { password: d.password } : {}),
  }
}
