import type { ReactNode } from 'react'
import { ActionableNotification, InlineNotification, SkeletonText, Tile } from '@carbon/react'
import { useApp } from '../shell/AppContext'

/** Error with an optional retry; the message comes from the server and is user-safe. */
export function ErrorNotice({
  title = 'Could not load this data',
  error,
  onRetry,
}: {
  title?: string
  error: Error
  onRetry?: () => void
}) {
  if (!onRetry) {
    return (
      <InlineNotification
        className="wide-notification"
        kind="error"
        lowContrast
        hideCloseButton
        title={title}
        subtitle={error.message}
      />
    )
  }
  return (
    <ActionableNotification
      hasFocus={false}
      inline
      className="wide-notification"
      kind="error"
      lowContrast
      hideCloseButton
      title={title}
      subtitle={error.message}
      actionButtonLabel="Try again"
      onActionButtonClick={onRetry}
    />
  )
}

export function LoadingBlock({ label = 'Loading' }: { label?: string }) {
  return (
    <div role="status" aria-live="polite" className="loading-block">
      <span className="visually-hidden">{label}</span>
      <SkeletonText paragraph lineCount={3} />
    </div>
  )
}

/** Shown on every screen that needs an environment when none exists. */
export function NeedsEnvironment() {
  const { openSettings } = useApp()
  return (
    <ActionableNotification
      hasFocus={false}
      inline
      className="wide-notification"
      kind="info"
      lowContrast
      hideCloseButton
      title="Add an environment first"
      subtitle="An environment holds the B2B host, instance ID and API key this tool works against."
      actionButtonLabel="Add environment"
      onActionButtonClick={openSettings}
    />
  )
}

export function EmptyState({
  icon,
  title,
  children,
}: {
  icon: ReactNode
  title: string
  children?: ReactNode
}) {
  return (
    <Tile className="empty-state">
      {icon}
      <h2>{title}</h2>
      <p>{children}</p>
    </Tile>
  )
}
