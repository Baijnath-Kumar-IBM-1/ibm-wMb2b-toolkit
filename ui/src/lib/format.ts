const dateTime = new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' })

export function formatDateTime(iso: string): string {
  const d = new Date(iso)
  return Number.isNaN(d.getTime()) ? iso : dateTime.format(d)
}

/** Narrow `unknown` event data to an object without trusting its shape. */
export function asRecord(value: unknown): Record<string, unknown> {
  return typeof value === 'object' && value !== null ? (value as Record<string, unknown>) : {}
}

export function errorMessage(e: unknown): string {
  return e instanceof Error ? e.message : 'Something went wrong'
}

export function controlNumber(n: number): string {
  return String(n).padStart(9, '0')
}

export function segmentCount(payload: string): number {
  return payload.split('~').filter((s) => s.trim() !== '').length
}

/** A random lowercase hex string, for unique sample names. */
export function randomHex(bytes: number): string {
  const buf = new Uint8Array(bytes)
  crypto.getRandomValues(buf)
  return Array.from(buf, (b) => b.toString(16).padStart(2, '0')).join('')
}

/** A random password that satisfies the server rules (8 to 128 characters). */
export function randomPassword(): string {
  return `Tr-${randomHex(12)}`
}

/** A random 9-digit number as a sample DUNS. */
export function randomDuns(): string {
  const buf = new Uint32Array(1)
  crypto.getRandomValues(buf)
  return String(100_000_000 + (buf[0] % 900_000_000))
}

export function downloadText(filename: string, text: string): void {
  const url = URL.createObjectURL(new Blob([text], { type: 'text/plain' }))
  const a = document.createElement('a')
  a.href = url
  a.download = filename
  document.body.append(a)
  a.click()
  a.remove()
  URL.revokeObjectURL(url)
}
