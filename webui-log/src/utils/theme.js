// Light / dark / follow the system (default). The choice is kept in this browser only;
// `data-theme` on <html> always carries the resolved 'light' or 'dark'.
export const THEME_STORAGE_KEY = 'rtdns-theme'
export const THEME_MODES = Object.freeze(['system', 'light', 'dark'])
export const THEME_LABELS = Object.freeze({ system: '跟随系统', light: '浅色', dark: '深色' })

export function normalizeThemeMode(value) {
  return THEME_MODES.includes(value) ? value : 'system'
}

export function resolveTheme(mode, prefersDark) {
  const normalized = normalizeThemeMode(mode)
  if (normalized === 'system') return prefersDark ? 'dark' : 'light'
  return normalized
}

function readStoredMode(storage) {
  try { return normalizeThemeMode(storage?.getItem(THEME_STORAGE_KEY)) } catch { return 'system' }
}

// One controller per page: applies the stored mode, follows the system while the mode is
// 'system', and saves a new choice.
export function createThemeController({ root, storage, matchMedia } = {}) {
  const query = typeof matchMedia === 'function' ? matchMedia('(prefers-color-scheme: dark)') : null
  let mode = readStoredMode(storage)
  const apply = () => {
    const theme = resolveTheme(mode, Boolean(query?.matches))
    root?.setAttribute('data-theme', theme)
    return theme
  }
  query?.addEventListener?.('change', () => { if (mode === 'system') apply() })
  apply()
  return {
    get mode() { return mode },
    setMode(next) {
      mode = normalizeThemeMode(next)
      try { storage?.setItem(THEME_STORAGE_KEY, mode) } catch { /* private mode: keep it for this page */ }
      return apply()
    }
  }
}
