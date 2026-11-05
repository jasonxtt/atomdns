import { defineConfig } from 'vitepress'

export default defineConfig({
  lang: 'zh-CN',
  title: 'AtomDNS',
  description: '用 Rust 重写的 MosDNS：带完整 WebUI 的家庭网络 DNS 分流器',
  base: '/atomdns/',
  cleanUrls: true,
  lastUpdated: true,
  head: [['link', { rel: 'icon', href: '/atomdns/logo.png' }]],

  themeConfig: {
    logo: '/logo.png',
    nav: [
      { text: '指南', link: '/guide/quick-start', activeMatch: '/guide/' },
      { text: '插件', link: '/plugins/', activeMatch: '/plugins/' },
      { text: '更新日志', link: 'https://github.com/jasonxtt/atomdns/releases' },
    ],

    sidebar: {
      '/guide/': [
        {
          text: '入门',
          items: [
            { text: '快速开始', link: '/guide/quick-start' },
            { text: 'WebUI 导览', link: '/guide/webui' },
            { text: '从 MosDNS 迁移', link: '/guide/migrate' },
          ],
        },
        {
          text: '默认配置',
          items: [
            { text: '分流流程', link: '/guide/flow' },
            { text: '运行模式', link: '/guide/modes' },
            { text: '专属分流组', link: '/guide/special-groups' },
            { text: '路由学习', link: '/guide/route-learning' },
            { text: '功能开关', link: '/guide/flags' },
          ],
        },
        {
          text: '配置参考',
          items: [
            { text: '配置文件结构', link: '/guide/config' },
            { text: 'sequence 语法', link: '/plugins/sequence' },
          ],
        },
        {
          text: '运维',
          items: [
            { text: '在线更新与回滚', link: '/guide/update' },
            { text: '纯后端运行', link: '/guide/headless' },
            { text: '自定义 WebUI', link: '/guide/custom-ui' },
            { text: '常见问题', link: '/guide/faq' },
          ],
        },
      ],
      '/plugins/': [
        { text: '总览', items: [{ text: '插件一览', link: '/plugins/' }] },
        { text: '服务器', items: [{ text: 'udp_server / tcp_server', link: '/plugins/server' }] },
        {
          text: '上游与缓存',
          items: [
            { text: 'forward / aliapi', link: '/plugins/forward' },
            { text: 'cache', link: '/plugins/cache' },
            { text: 'fallback', link: '/plugins/fallback' },
            { text: 'ecs_handler', link: '/plugins/ecs_handler' },
            { text: 'socks5_proxy / ecs_address', link: '/plugins/settings-objects' },
          ],
        },
        {
          text: '数据',
          items: [
            { text: 'domain_set', link: '/plugins/domain_set' },
            { text: 'ip_set', link: '/plugins/ip_set' },
            { text: 'domain_mapper', link: '/plugins/domain_mapper' },
            { text: 'adguard_rule', link: '/plugins/adguard_rule' },
            { text: 'hosts', link: '/plugins/hosts' },
            { text: 'redirect', link: '/plugins/redirect' },
          ],
        },
        {
          text: '流程与标记',
          items: [
            { text: 'sequence', link: '/plugins/sequence' },
            { text: 'fast_mark', link: '/plugins/fast_mark' },
            { text: 'flow_setter / tag_setter', link: '/plugins/flow_setter' },
            { text: 'feature_flag', link: '/plugins/feature_flag' },
          ],
        },
        {
          text: 'AtomDNS 扩展',
          items: [
            { text: 'route_learner', link: '/plugins/route_learner' },
            { text: 'requery', link: '/plugins/requery' },
            { text: 'rewrite', link: '/plugins/rewrite' },
            { text: 'domain_output', link: '/plugins/domain_output' },
            { text: 'webinfo', link: '/plugins/webinfo' },
          ],
        },
        {
          text: '速查',
          items: [
            { text: '匹配器', link: '/plugins/matchers' },
            { text: '执行器', link: '/plugins/executors' },
          ],
        },
      ],
    },

    socialLinks: [{ icon: 'github', link: 'https://github.com/jasonxtt/atomdns' }],
    search: {
      provider: 'local',
      options: {
        translations: {
          button: { buttonText: '搜索', buttonAriaLabel: '搜索' },
          modal: {
            noResultsText: '没有找到',
            resetButtonTitle: '清除',
            footer: { selectText: '选择', navigateText: '切换', closeText: '关闭' },
          },
        },
      },
    },
    editLink: {
      pattern: 'https://github.com/jasonxtt/atomdns/edit/main/docs/site/:path',
      text: '在 GitHub 上编辑此页',
    },
    outline: { label: '本页目录', level: [2, 3] },
    docFooter: { prev: '上一页', next: '下一页' },
    lastUpdated: { text: '最后更新' },
    returnToTopLabel: '回到顶部',
    sidebarMenuLabel: '目录',
    darkModeSwitchLabel: '外观',
    lightModeSwitchTitle: '切换到浅色',
    darkModeSwitchTitle: '切换到深色',
    footer: {
      message: '基于 GPL-3.0 发布',
      copyright: 'AtomDNS · 致谢 MosDNS 及其维护者',
    },
  },
})
