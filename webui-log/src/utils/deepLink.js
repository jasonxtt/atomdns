// Landing on a search hit: scroll the element into view and flash it.
export function flashElement(element) {
  if (!element) return false
  element.scrollIntoView?.({ block: 'center', behavior: 'smooth' })
  element.classList.add('rt-found')
  setTimeout(() => element.classList.remove('rt-found'), 2400)
  return true
}

// The page may still be loading its rows: try a few times.
export function flashSelector(selector, tries = 12) {
  if (typeof document === 'undefined') return
  if (flashElement(document.querySelector(selector))) return
  if (tries > 0) setTimeout(() => flashSelector(selector, tries - 1), 150)
}

export function queryText(value) {
  return String(Array.isArray(value) ? value[0] || '' : value || '')
}
