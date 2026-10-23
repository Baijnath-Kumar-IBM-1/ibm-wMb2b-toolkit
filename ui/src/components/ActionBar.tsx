import type { ReactNode } from 'react'

export function ActionBar({
  icon,
  title,
  detail,
  children,
}: {
  icon: ReactNode
  title: string
  detail: string
  children: ReactNode
}) {
  return (
    <div className="action-bar">
      <div className="action-summary">
        {icon}
        <div>
          <strong>{title}</strong>
          <span>{detail}</span>
        </div>
      </div>
      <div className="action-buttons">{children}</div>
    </div>
  )
}
