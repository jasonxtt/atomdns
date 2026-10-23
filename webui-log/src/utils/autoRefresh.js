// Auto refresh of the current page, kept in this browser only.
export const AUTO_REFRESH_STORAGE_KEY = 'rtdns-auto-refresh'

export function normalizeAutoRefreshState(state = {}) {
  return {
    enabled: Boolean(state?.enabled),
    intervalSeconds: Math.max(5, Number(state?.intervalSeconds || 15))
  }
}

export function loadAutoRefreshState() {
  try {
    return normalizeAutoRefreshState(JSON.parse(localStorage.getItem(AUTO_REFRESH_STORAGE_KEY) || 'null') || {})
  } catch {
    return normalizeAutoRefreshState()
  }
}

// Saves the state and tells the shell to restart its timer.
export function saveAutoRefreshState(state) {
  const normalized = normalizeAutoRefreshState(state)
  try { localStorage.setItem(AUTO_REFRESH_STORAGE_KEY, JSON.stringify(normalized)) } catch { /* kept for this page only */ }
  window.dispatchEvent(new CustomEvent('mosdns-auto-refresh-update', { detail: normalized }))
  return normalized
}
