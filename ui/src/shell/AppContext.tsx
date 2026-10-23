import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from 'react'
import { api, unwrap } from '../api/client'
import { subscribeEvents, type ServerEvent } from '../api/events'
import type { Environment, Pack } from '../api/types'
import { useLoad, type Loaded } from '../lib/useLoad'

export type Toast = { kind: 'success' | 'error'; message: string }
export type EventHandler = (event: ServerEvent) => void

export interface AppContextValue {
  environments: Loaded<Environment[]>
  /** The environment all screens work against; undefined until one exists. */
  selected: Environment | undefined
  selectEnvironment: (id: string) => void
  packs: Loaded<Pack[]>
  notify: (message: string, kind?: Toast['kind']) => void
  toast: Toast | null
  dismissToast: () => void
  settingsOpen: boolean
  openSettings: () => void
  closeSettings: () => void
  navigate: (featureId: string) => void
  /** Registers a listener for server events; returns the unsubscribe function. */
  onEvent: (handler: EventHandler) => () => void
}

const Context = createContext<AppContextValue | null>(null)

export function useApp(): AppContextValue {
  const value = useContext(Context)
  if (!value) throw new Error('useApp must be used inside <AppProvider>')
  return value
}

/** Calls `handler` for every server event while the component is mounted. */
export function useServerEvents(handler: EventHandler): void {
  const { onEvent } = useApp()
  const latest = useRef(handler)
  latest.current = handler
  useEffect(() => onEvent((e) => latest.current(e)), [onEvent])
}

export function AppProvider({
  navigate,
  children,
}: {
  navigate: (featureId: string) => void
  children: ReactNode
}) {
  const environments = useLoad(() => unwrap(api.GET('/environments')), [])
  const packs = useLoad(() => unwrap(api.GET('/packs')), [])
  const [selectedId, setSelectedId] = useState('')
  const [toast, setToast] = useState<Toast | null>(null)
  const [settingsOpen, setSettingsOpen] = useState(false)
  const handlers = useRef(new Set<EventHandler>())
  const toastTimer = useRef<number | undefined>(undefined)

  const list = environments.data
  const selected = useMemo(
    () => list?.find((e) => e.id === selectedId) ?? list?.find((e) => e.is_default) ?? list?.[0],
    [list, selectedId],
  )

  const { reload } = environments
  useEffect(() => {
    return subscribeEvents(
      (event) => {
        if (event.kind.startsWith('environment.')) void reload()
        handlers.current.forEach((h) => h(event))
      },
      () => void reload(),
    )
  }, [reload])

  const notify = useCallback((message: string, kind: Toast['kind'] = 'success') => {
    setToast({ kind, message })
    window.clearTimeout(toastTimer.current)
    toastTimer.current = window.setTimeout(() => setToast(null), 4000)
  }, [])

  const onEvent = useCallback((handler: EventHandler) => {
    handlers.current.add(handler)
    return () => {
      handlers.current.delete(handler)
    }
  }, [])

  const value = useMemo<AppContextValue>(
    () => ({
      environments,
      selected,
      selectEnvironment: setSelectedId,
      packs,
      notify,
      toast,
      dismissToast: () => setToast(null),
      settingsOpen,
      openSettings: () => setSettingsOpen(true),
      closeSettings: () => setSettingsOpen(false),
      navigate,
      onEvent,
    }),
    [environments, selected, packs, notify, toast, settingsOpen, navigate, onEvent],
  )

  return <Context.Provider value={value}>{children}</Context.Provider>
}
