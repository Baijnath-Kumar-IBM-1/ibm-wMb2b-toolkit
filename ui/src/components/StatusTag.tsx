import { Tag } from '@carbon/react'

type Kind = 'succeeded' | 'failed' | 'pending' | 'skipped' | 'running' | 'notrun'

const TAGS: Record<Kind, { type: 'green' | 'red' | 'cool-gray' | 'blue' | 'gray'; text: string }> = {
  succeeded: { type: 'green', text: 'Done' },
  failed: { type: 'red', text: 'Failed' },
  pending: { type: 'cool-gray', text: 'Pending' },
  skipped: { type: 'gray', text: 'Already done' },
  running: { type: 'blue', text: 'Running' },
  notrun: { type: 'cool-gray', text: 'Not run' },
}

export function StatusTag({ kind }: { kind: Kind }) {
  const { type, text } = TAGS[kind]
  return <Tag type={type}>{text}</Tag>
}
