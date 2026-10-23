import type { SubmissionSummary } from '../../api/types'

const COLUMNS = [
  'submitted_at',
  'domain',
  'transaction_code',
  'result',
  'http_status',
  'isa_control',
  'gs_control',
  'duration_ms',
  'trace_values',
  'response_excerpt',
  'id',
] as const

/**
 * Spreadsheets run a cell that starts with = + - @ as a formula. Response
 * text comes from a remote system, so such cells are prefixed with an
 * apostrophe before they are written.
 */
function csvCell(value: unknown): string {
  let text = value == null ? '' : typeof value === 'object' ? JSON.stringify(value) : String(value)
  if (/^[=+\-@\t\r]/.test(text)) text = `'${text}`
  return /[",\r\n]/.test(text) ? `"${text.replace(/"/g, '""')}"` : text
}

export function toCsv(rows: SubmissionSummary[]): string {
  const lines = rows.map((row) => COLUMNS.map((c) => csvCell(row[c])).join(','))
  return [COLUMNS.join(','), ...lines].join('\r\n') + '\r\n'
}

export function toJson(rows: SubmissionSummary[]): string {
  return JSON.stringify(rows, null, 2) + '\n'
}
