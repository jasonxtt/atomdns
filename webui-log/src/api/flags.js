// The named feature flags (`GET /api/v1/flags`): which page shows which flag, and
// how a failed change is explained. The flag's own `section` decides the page.

// Flag section → sub-item of 规则设置 (`rewrite` flags belong to 分流规则 now).
const PAGE_OF_SECTION = Object.freeze({
  mode: 'mode',
  upstream: 'upstream',
  routing: 'routing',
  rewrite: 'routing',
  blocking: 'blocking',
  learning: 'learning',
  cache: 'cache',
  client: 'clients',
  special_groups: 'special-groups'
})

export function pageOfFlag(flag) {
  return PAGE_OF_SECTION[flag?.section] || ''
}

// The flags of one page in the order the configuration declares them.
export function flagsForPage(flags, page) {
  return (Array.isArray(flags) ? flags : []).filter((flag) => pageOfFlag(flag) === page)
}

export function isValidFlagList(data) {
  return Boolean(data) && Array.isArray(data.flags) && data.flags.every((flag) => flag && typeof flag.key === 'string')
}

export function optionLabel(flag, value) {
  return (flag?.options || []).find((option) => option.value === value)?.label || String(value)
}

// What the user is told after a change was applied.
export function changeMessage(flag, result) {
  const name = flag?.label || flag?.key || '开关'
  return result?.requery_triggered
    ? `“${name}”已切换，正在刷新缓存与预热`
    : `“${name}”已切换`
}

// A failed change: 409 means another change was being applied (the page reads the
// flags again), 503 carries the reason (an online update is being tried out).
export function changeFailure(flag, error) {
  const name = flag?.label || flag?.key || '开关'
  const reason = String(error?.message || '').trim()
  if (error?.status === 409) return { reload: true, message: `“${name}”没有切换：配置正在被另一项修改占用，已重新读取，请再点一次` }
  if (error?.status === 503) return { reload: false, message: reason || `“${name}”暂时不能切换：配置更新试运行中` }
  return { reload: true, message: `切换“${name}”失败: ${reason || '未知错误'}` }
}

// Runs one change request and says what to show and whether the page should read the
// flags again. Never throws.
export async function runFlagChange(flag, request) {
  try {
    const result = await request()
    return { ok: true, message: changeMessage(flag, result), reload: true }
  } catch (error) {
    const failure = changeFailure(flag, error)
    return { ok: false, message: failure.message, reload: failure.reload }
  }
}
