import { h } from 'vue'
import LogsDiagnosticPage from './LogsDiagnosticPage.vue'
import LogsPage from './LogsPage.vue'
import OverviewPage from './OverviewPage.vue'
import RulesBlockingPage from './rules/RulesBlockingPage.vue'
import RulesCachePage from './rules/RulesCachePage.vue'
import RulesLearningPage from './rules/RulesLearningPage.vue'
import RulesPlaceholderPage from './rules/RulesPlaceholderPage.vue'
import RulesRewritePage from './rules/RulesRewritePage.vue'
import RulesRoutingPage from './rules/RulesRoutingPage.vue'
import RulesUpstreamPage from './rules/RulesUpstreamPage.vue'
import SystemPage from './system/SystemPage.vue'

const placeholder = (title, description) => ({ name: 'RulesPlaceholder', render: () => h(RulesPlaceholderPage, { title, description }) })
const withProps = (component, props) => ({ name: component.name, render: () => h(component, props) })

// Route name (see navigation.js) → page component.
const PAGE_COMPONENTS = {
  overview: OverviewPage,
  logs: LogsPage,
  'logs-diagnostic': LogsDiagnosticPage,
  'rules-mode': placeholder('运行模式', '核心运行模式、DNS 分流模式等开关将按新的功能开关接口（/api/v1/flags）在这里重做。'),
  'rules-upstream': RulesUpstreamPage,
  'rules-routing': RulesRoutingPage,
  'rules-blocking': RulesBlockingPage,
  'rules-rewrite': RulesRewritePage,
  'rules-learning': RulesLearningPage,
  'rules-cache': RulesCachePage,
  'rules-clients': placeholder('客户端', '客户端 IP 名单目前在 分流规则 › 本地规则 里编辑，客户端别名在 日志 页。'),
  'rules-special-groups': withProps(RulesUpstreamPage, { specialGroups: true }),
  system: SystemPage
}

export function pageComponent(name) {
  const component = PAGE_COMPONENTS[name]
  if (!component) throw new Error(`no page component for route ${name}`)
  return component
}
