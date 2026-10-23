import { createRouter, createWebHashHistory } from 'vue-router'
import { NAV_MODULES, PAGES } from './navigation.js'

// `pageComponent(name)` returns the component of a page; tests pass a stub so the
// routing table can be checked without compiling the pages.
export function createAppRouter({ history = createWebHashHistory(), pageComponent }) {
  const routes = [
    { path: '/', redirect: '/overview' },
    ...NAV_MODULES.filter(module => module.items.length > 0)
      .map(module => ({ path: `/${module.id}`, redirect: module.path })),
    ...PAGES.map(page => ({ path: page.path, name: page.name, component: pageComponent(page.name), meta: { title: page.title } })),
    // The former sub-pages of 系统管理 are sections of one page now.
    { path: '/system/:section(.*)*', redirect: '/system' },
    // Unknown addresses go back to the overview.
    { path: '/:unknown(.*)*', redirect: '/overview' }
  ]
  return createRouter({ history, routes })
}
