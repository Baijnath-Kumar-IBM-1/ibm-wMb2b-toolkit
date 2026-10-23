import { Locked } from '@carbon/icons-react'
import { EmptyState } from '../components/States'

/** Shown when the API token is missing or was rejected. */
export function Unauthorized() {
  return (
    <main className="unauthorized">
      <EmptyState icon={<Locked size={32} />} title="Open the link printed by the server">
        This page needs the access link that <code>trace-server</code> prints when it starts, which ends in{' '}
        <code>#token=…</code>. Open that link in your browser, or restart the server to get a new one. In the
        desktop app, restart the app.
      </EmptyState>
    </main>
  )
}
