// Pages with editable lists report unsaved changes here; the router asks before
// leaving a page and the browser asks before the tab closes.

const dirty = new Map()

export function setDirty(key, value) {
  if (value) dirty.set(key, true)
  else dirty.delete(key)
}

export function clearDirty(key) {
  dirty.delete(key)
}

export function hasUnsaved() {
  return dirty.size > 0
}

export function resetUnsaved() {
  dirty.clear()
}

export function leaveMessage() {
  return '页面上有未保存的改动，离开后这些改动会丢失。确定离开吗？'
}

export function installBeforeUnload(target = window) {
  const handler = (event) => {
    if (!hasUnsaved()) return
    event.preventDefault()
    event.returnValue = ''
  }
  target.addEventListener('beforeunload', handler)
  return () => target.removeEventListener('beforeunload', handler)
}
