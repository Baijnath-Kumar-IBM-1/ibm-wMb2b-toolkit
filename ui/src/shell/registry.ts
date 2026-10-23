import { activityFeature } from '../features/activity'
import { documentsFeature } from '../features/documents'
import { overviewFeature } from '../features/overview'
import { setupFeature } from '../features/setup'
import { studioFeature } from '../features/studio'
import type { Feature } from './feature'

/**
 * The single place that lists screens, in navigation order. To add a screen,
 * create its `Feature` and append it here (see docs/adding-a-feature.md).
 */
export const features: Feature[] = [overviewFeature, setupFeature, documentsFeature, studioFeature, activityFeature]
