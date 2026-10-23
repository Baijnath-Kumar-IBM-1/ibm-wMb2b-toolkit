import type { ComponentType } from 'react'
import type { CarbonIconType } from '@carbon/icons-react'

/**
 * A screen that plugs into the shell. Add one by creating
 * `features/<name>/index.tsx` that exports a `Feature` and listing it in
 * `registry.ts`; navigation and the page header are generated from this.
 */
export interface Feature {
  id: string
  /** Side navigation text. */
  label: string
  icon: CarbonIconType
  title: string
  subtitle: string
  Component: ComponentType
}
