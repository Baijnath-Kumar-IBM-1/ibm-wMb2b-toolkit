import { useCallback, useEffect, useState } from 'react'
import { UNAUTHORIZED_EVENT } from './api/client'
import { getToken } from './api/token'
import { AppProvider } from './shell/AppContext'
import { features } from './shell/registry'
import { Shell } from './shell/Shell'
import { Unauthorized } from './shell/Unauthorized'

export default function App() {
  const [authorized, setAuthorized] = useState(() => getToken() !== null)
  const [activeId, setActiveId] = useState(features[0].id)
  const navigate = useCallback((id: string) => {
    if (features.some((f) => f.id === id)) setActiveId(id)
  }, [])

  useEffect(() => {
    const onUnauthorized = () => setAuthorized(false)
    window.addEventListener(UNAUTHORIZED_EVENT, onUnauthorized)
    return () => window.removeEventListener(UNAUTHORIZED_EVENT, onUnauthorized)
  }, [])

  if (!authorized) return <Unauthorized />
  return (
    <AppProvider navigate={navigate}>
      <Shell activeId={activeId} onNavigate={navigate} />
    </AppProvider>
  )
}
