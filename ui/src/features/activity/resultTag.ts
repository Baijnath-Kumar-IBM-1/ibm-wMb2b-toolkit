import type { SubmissionSummary } from '../../api/types'

export const RESULT_TAG: Record<
  SubmissionSummary['result'],
  { type: 'green' | 'red' | 'cool-gray'; text: string }
> = {
  success: { type: 'green', text: 'Submitted' },
  failed: { type: 'red', text: 'Failed' },
  generated: { type: 'cool-gray', text: 'Generated only' },
}
