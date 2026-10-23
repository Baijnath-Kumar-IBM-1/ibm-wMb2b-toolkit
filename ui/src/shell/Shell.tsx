import { useEffect, useState } from 'react'
import {
  Header,
  HeaderGlobalAction,
  HeaderGlobalBar,
  HeaderMenuButton,
  HeaderName,
  SideNav,
  SideNavItems,
  SideNavLink,
  SkipToContent,
  Tag,
  ToastNotification,
} from '@carbon/react'
import { ChevronLeft, ChevronRight, Settings } from '@carbon/icons-react'
import { EnvironmentModal } from '../features/environments/EnvironmentModal'
import { useApp } from './AppContext'
import { features } from './registry'

export function Shell({ activeId, onNavigate }: { activeId: string; onNavigate: (id: string) => void }) {
  const { selected, openSettings, toast, dismissToast, environments } = useApp()
  const [sideNavOpen, setSideNavOpen] = useState(true)
  const active = features.find((f) => f.id === activeId) ?? features[0]
  const Screen = active.Component

  // Land on the environment dialog the first time, when there is nothing to work with.
  const noEnvironments = environments.data?.length === 0
  useEffect(() => {
    if (noEnvironments) openSettings()
  }, [noEnvironments]) // eslint-disable-line react-hooks/exhaustive-deps

  function go(id: string) {
    onNavigate(id)
    if (window.innerWidth < 1056) setSideNavOpen(false)
  }

  return (
    <div className={`carbon-app ${sideNavOpen ? '' : 'side-nav-collapsed'}`}>
      <Header aria-label="Trace">
        <SkipToContent href="#main-content" />
        <HeaderMenuButton
          aria-label={sideNavOpen ? 'Close navigation' : 'Open navigation'}
          isActive={sideNavOpen}
          onClick={() => setSideNavOpen(!sideNavOpen)}
        />
        <HeaderName href="#" prefix="" onClick={(e: React.MouseEvent) => e.preventDefault()}>
          <img className="app-logo" src="/logo.png" alt="" />
          <strong>Trace</strong>
        </HeaderName>
        <HeaderGlobalBar>
          <HeaderGlobalAction aria-label="Environment settings" tooltipAlignment="end" onClick={openSettings}>
            <Settings size={20} />
          </HeaderGlobalAction>
        </HeaderGlobalBar>
      </Header>

      <SideNav
        aria-label="Application navigation"
        expanded={sideNavOpen}
        isPersistent={false}
        onOverlayClick={() => setSideNavOpen(false)}
      >
        <SideNavItems>
          <div className="side-nav-heading">Workspace</div>
          {features.map((f) => (
            <SideNavLink
              key={f.id}
              href="#"
              renderIcon={f.icon}
              isActive={f.id === active.id}
              aria-current={f.id === active.id ? 'page' : undefined}
              onClick={(e: React.MouseEvent) => {
                e.preventDefault()
                go(f.id)
              }}
            >
              {f.label}
            </SideNavLink>
          ))}
          <div className="side-nav-bottom">
            <SideNavLink
              href="#"
              renderIcon={sideNavOpen ? ChevronLeft : ChevronRight}
              onClick={(e: React.MouseEvent) => {
                e.preventDefault()
                setSideNavOpen(!sideNavOpen)
              }}
            >
              {sideNavOpen ? 'Collapse navigation' : 'Expand navigation'}
            </SideNavLink>
            <div className="side-nav-divider" />
            <SideNavLink
              href="#"
              renderIcon={Settings}
              onClick={(e: React.MouseEvent) => {
                e.preventDefault()
                openSettings()
              }}
            >
              Environment settings
            </SideNavLink>
            <div className="environment-status">
              <span className={selected ? 'status status-ready' : 'status'} aria-hidden="true" />
              <div>
                <strong>{selected ? 'Environment selected' : 'Not configured'}</strong>
                <span>{selected?.name ?? 'Add an environment to begin'}</span>
              </div>
            </div>
          </div>
        </SideNavItems>
      </SideNav>

      <main className="workspace" id="main-content">
        <section className="page-header">
          <div>
            <p className="breadcrumb">Trace / {active.label}</p>
            <h1>{active.title}</h1>
            <p>{active.subtitle}</p>
          </div>
          <Tag type={selected ? 'green' : 'warm-gray'}>{selected?.name ?? 'No environment'}</Tag>
        </section>
        <section className="page-content">
          <Screen key={active.id} />
        </section>
      </main>

      <EnvironmentModal />

      {toast && (
        <div className="toast-wrap" role="status" aria-live="polite">
          <ToastNotification
            kind={toast.kind === 'error' ? 'error' : 'success'}
            lowContrast
            timeout={0}
            title={toast.message}
            onClose={dismissToast}
          />
        </div>
      )}
    </div>
  )
}
