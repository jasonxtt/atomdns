import { createRouter, createWebHashHistory } from 'vue-router'
import { NAV_MODULES, PAGES, REDIRECTS } from './navigation.js'
import { hasUnsaved } from './utils/unsaved.js'

// `pageComponent(name)` returns the component of a page; tests pass a stub so the
// routing table can be checked without compiling the pages. `confirmLeave()` asks
// whether a page with unsaved changes may be left (true: leave).
export function createAppRouter({ history = createWebHashHistory(), pageComponent, confirmLeave = async () => true }) {
  const routes = [
    { path: '/', redirect: '/overview' },
    ...NAV_MODULES.filter(module => module.items.length > 0)
      .map(module => ({ path: `/${module.id}`, redirect: module.path })),
    ...REDIRECTS.map(({ from, to }) => ({ path: from, redirect: to })),
    ...PAGES.map(page => ({ path: page.path, name: page.name, component: pageComponent(page.name), meta: { title: page.title } })),
    // The former sub-pages of 系统管理 are sections of one page now.
    { path: '/system/:section(.*)*', redirect: '/system' },
    // Unknown addresses go back to the overview.
    { path: '/:unknown(.*)*', redirect: '/overview' }
  ]
  const router = createRouter({ history, routes })
  // Moving to another page, sub-item or special group (the query) with unsaved changes
  // asks first.
  router.beforeEach(async (to, from) => {
    if (to.fullPath === from.fullPath || !hasUnsaved()) return true
    return (await confirmLeave()) ? true : false
  })
  return router
}
