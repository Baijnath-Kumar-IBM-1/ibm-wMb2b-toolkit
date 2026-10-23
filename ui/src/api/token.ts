/**
 * API token handling.
 *
 * - Web: the server prints `http://127.0.0.1:<port>/#token=...`. The token
 *   lives in the URL *fragment*, which the browser never sends to a server,
 *   so it stays out of request lines and logs. It is moved to
 *   `sessionStorage` and removed from the address bar immediately.
 * - Desktop (Tauri): the shell sets `window.__TRACE_TOKEN__` before the page
 *   loads; nothing is stored.
 *
 * The token is only ever sent in the `Authorization` header.
 */

declare global {
  interface Window {
    __TRACE_TOKEN__?: string
  }
}

const STORAGE_KEY = 'trace.token'

let cached: string | null = null

export function initToken(): void {
  const injected = window.__TRACE_TOKEN__
  if (injected) {
    cached = injected
    return
  }
  const match = /(?:^|[#&])token=([^&]+)/.exec(window.location.hash)
  if (match) {
    cached = decodeURIComponent(match[1])
    sessionStorage.setItem(STORAGE_KEY, cached)
    // Drop the token from the address bar and history entry.
    history.replaceState(null, '', window.location.pathname + window.location.search)
    return
  }
  cached = sessionStorage.getItem(STORAGE_KEY)
}

export function getToken(): string | null {
  return cached
}

export function clearToken(): void {
  cached = null
  sessionStorage.removeItem(STORAGE_KEY)
}
