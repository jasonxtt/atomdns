import LogsDiagnosticPage from './LogsDiagnosticPage.vue'
import LogsPage from './LogsPage.vue'
import OverviewPage from './OverviewPage.vue'
import RulesBlockingPage from './rules/RulesBlockingPage.vue'
import RulesCachePage from './rules/RulesCachePage.vue'
import RulesClientsPage from './rules/RulesClientsPage.vue'
import RulesLearningPage from './rules/RulesLearningPage.vue'
import RulesModePage from './rules/RulesModePage.vue'
import RulesRoutingPage from './rules/RulesRoutingPage.vue'
import RulesSpecialGroupsPage from './rules/RulesSpecialGroupsPage.vue'
import RulesUpstreamPage from './rules/RulesUpstreamPage.vue'
import SystemPage from './system/SystemPage.vue'

// Route name (see navigation.js) → page component.
const PAGE_COMPONENTS = {
  overview: OverviewPage,
  logs: LogsPage,
  'logs-diagnostic': LogsDiagnosticPage,
  'rules-mode': RulesModePage,
  'rules-upstream': RulesUpstreamPage,
  'rules-routing': RulesRoutingPage,
  'rules-blocking': RulesBlockingPage,
  'rules-learning': RulesLearningPage,
  'rules-cache': RulesCachePage,
  'rules-clients': RulesClientsPage,
  'rules-special-groups': RulesSpecialGroupsPage,
  system: SystemPage
}

export function pageComponent(name) {
  const component = PAGE_COMPONENTS[name]
  if (!component) throw new Error(`no page component for route ${name}`)
  return component
}
