import test from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'

import { OPERATION_IDS, requestOperation, validateRuntimeCapabilities } from '../src/api/runtimeCapabilities.js'

const read = (path) => readFileSync(new URL(path, import.meta.url), 'utf8')

test('the system operations are gated by the paths the native host serves', () => {
  assert.equal(requestOperation('/api/v1/system/webui-port', 'POST'), 'system.webui_port')
  assert.equal(requestOperation('/api/v1/system/restart', 'POST'), 'system.restart')
  assert.equal(requestOperation('/api/v1/config/export', 'POST'), 'system.config_management')
  // The process figures travel on the metrics endpoint the cache panel already uses.
  assert.equal(requestOperation('/metrics', 'GET'), 'metrics.cache')
})

test('a native peer that reports the operations supported enables them without reasons', () => {
  const ids = ['system.restart', 'system.webui_port', 'metrics.process']
  const value = validateRuntimeCapabilities({
    schema_version: 1,
    runtime: 'rust',
    special_groups: { enabled: false },
    upstream_protocols: [],
    rule_formats: [],
    unsupported_features: [],
    endpoints: {},
    ui_operations: Object.fromEntries(OPERATION_IDS.map((id) => [id, ids.includes(id) ? { supported: true, reason: null } : { supported: false, reason: 'n/a' }]))
  })
  for (const id of ids) assert.deepEqual(value.ui_operations[id], { supported: true, reason: null })
})

test('the system info and the overview show only the process figures the Rust runtime has', () => {
  const update = read('../src/pages/system/SystemUpdateSection.vue')
  const overview = read('../src/pages/OverviewPage.vue')
  assert.match(update, /supportsOperation\('metrics\.process'\)/)
  assert.match(overview, /supportsOperation\('metrics\.process'\)/)
  for (const source of [update, overview]) {
    assert.doesNotMatch(source, /go_|goroutine|goVersion|gcCount|Go 版本/, 'no Go-runtime figure is read or shown')
  }
  assert.match(read('../src/utils/overviewMetrics.js'), /atomdns_process_threads/)
})

test('the WebUI port, restart and reset statistics live on the service page', () => {
  const service = read('../src/pages/system/SystemServiceSection.vue')
  assert.match(service, /<CapabilityBoundary operation="system\.webui_port">\s*<SettingRow label="WebUI 端口"/)
  assert.match(service, /将清空概览页全部统计并重启 AtomDNS，是否继续？/)
  assert.match(service, /'\/api\/v1\/system\/restart'/)
  assert.doesNotMatch(read('../src/App.vue'), /重置统计/, 'the shell no longer carries the reset button')
})

test('the backup page only exports, under the atomdns file name, and nothing calls the removed endpoints', () => {
  const backup = read('../src/pages/system/SystemBackupSection.vue')
  assert.match(backup, /'\/api\/v1\/config\/export'/)
  assert.match(backup, /atomdns_backup\.zip/)
  for (const path of ['../src/pages/system/SystemBackupSection.vue', '../src/pages/system/SystemUiSection.vue', '../src/App.vue', '../src/main.js']) {
    const source = read(path)
    assert.doesNotMatch(source, /update_from_url|\/api\/v1\/appearance/, path)
  }
})

test('the update page shows the configuration package state from health and status', () => {
  const update = read('../src/pages/system/SystemUpdateSection.vue')
  for (const field of ['applied_config_schema', 'required_config_schema', 'config_update_status', 'rollback_performed', 'last_error']) {
    assert.match(update, new RegExp(field), field)
  }
  assert.doesNotMatch(update, /prefer_v3: preferV3|切换 v3/, 'no Go amd64 level switch')
})
