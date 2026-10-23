import { Select, SelectItem } from '@carbon/react'
import { useApp } from '../shell/AppContext'

/** Picks the environment every screen works against. */
export function EnvironmentSelect({ id }: { id: string }) {
  const { environments, selected, selectEnvironment } = useApp()
  const list = environments.data ?? []
  return (
    <Select
      id={id}
      labelText="Environment"
      value={selected?.id ?? ''}
      disabled={list.length === 0}
      onChange={(event) => selectEnvironment(event.target.value)}
    >
      {list.length === 0 && <SelectItem value="" text="No environment yet" />}
      {list.map((environment) => (
        <SelectItem
          key={environment.id}
          value={environment.id}
          text={`${environment.name}${environment.is_default ? ' (default)' : ''}`}
        />
      ))}
    </Select>
  )
}
