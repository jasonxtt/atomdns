// Helpers of the shared manual-list editor: reading `show` output, telling a
// real change from a re-ordered or re-spaced one, and the truncation guard.

// A list longer than this is shown cut off and cannot be saved from the page.
export const LIST_LINE_CAP = 20000

// `show` answers at most `limit` lines; asking for one more than the cap tells
// a list that fits from one that was cut.
export function showUrl(tag, cap = LIST_LINE_CAP) {
  return `/plugins/${encodeURIComponent(tag)}/show?limit=${cap + 1}`
}

export function splitRules(text) {
  return String(text || '')
    .split('\n')
    .map((line) => line.trim())
    .filter(Boolean)
}

// The server prints one accepted rule per line with a trailing newline, while
// the editor holds whatever was typed: comparing the rule lists keeps a trailing
// newline or a blank line from looking like a change.
export function canonicalRules(text) {
  return splitRules(text).join('\n')
}

export function sameRules(left, right) {
  return canonicalRules(left) === canonicalRules(right)
}

// What the editor shows for a `show` answer: the lines (at most `cap`) and
// whether the list was longer. A cut list must never be saved back.
export function readList(text, cap = LIST_LINE_CAP) {
  const lines = splitRules(text)
  if (lines.length > cap) return { text: lines.slice(0, cap).join('\n'), truncated: true, count: lines.length }
  return { text: lines.join('\n'), truncated: false, count: lines.length }
}

// The lines that contain `query` (case-insensitive); an empty query matches nothing
// because the editor shows the whole text then.
export function filterLines(text, query) {
  const needle = String(query || '').trim().toLowerCase()
  if (!needle) return []
  return splitRules(text).filter((line) => line.toLowerCase().includes(needle))
}

// A list can be saved when it was read completely and is not in an unconfirmed state.
export function canSaveList({ truncated, loading, saving, uncertain, dirty }) {
  if (truncated || loading || saving) return false
  return Boolean(dirty || uncertain)
}
