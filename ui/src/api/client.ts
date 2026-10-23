import createClient, { type Middleware } from 'openapi-fetch'
import type { components, paths } from './schema'
import { clearToken, getToken } from './token'

export type Schemas = components['schemas']

/** Fired on `window` when the server rejects the token. */
export const UNAUTHORIZED_EVENT = 'trace:unauthorized'

/** Error with the server's user-safe message and per-field problems. */
export class ApiFailure extends Error {
  readonly status: number
  readonly fields: { field: string; message: string }[]

  constructor(status: number, body: Partial<Schemas['ApiError']> | undefined) {
    super(body?.error ?? 'Request failed')
    this.name = 'ApiFailure'
    this.status = status
    this.fields = body?.fields ?? []
  }

  /** Message for one form field, if the server reported one. */
  fieldMessage(field: string): string | undefined {
    return this.fields.find((f) => f.field === field)?.message
  }
}

const auth: Middleware = {
  onRequest({ request }) {
    const token = getToken()
    if (token) request.headers.set('Authorization', `Bearer ${token}`)
    return request
  },
}

// Same origin in production and through the Vite proxy in development.
export const api = createClient<paths>({ baseUrl: '/api' })
api.use(auth)

/** Unwraps a typed response or throws an [ApiFailure]. */
export async function unwrap<T>(
  call: Promise<{ data?: T; error?: unknown; response: Response }>,
): Promise<T> {
  let result
  try {
    result = await call
  } catch {
    throw new ApiFailure(0, { error: 'Cannot reach the local server. Is it still running?' })
  }
  const { data, error, response } = result
  if (response.status === 401) {
    // The token is missing or stale (for example the server was restarted).
    clearToken()
    window.dispatchEvent(new Event(UNAUTHORIZED_EVENT))
  }
  if (!response.ok) {
    throw new ApiFailure(response.status, error as Partial<Schemas['ApiError']> | undefined)
  }
  return data as T
}
