import test from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { groupByName, latestDelay, nodeSource, validateNode, validateSubscription } from '../src/utils/singbox.js'
import { requestOperation } from '../src/api/runtimeCapabilities.js'

test('sing-box subscription and manual node validation reject malformed values', () => {
  assert.equal(validateSubscription({ name: 'airport', url: 'https://example.com/sub', update_interval_hours: 24 }), '')
  assert.match(validateSubscription({ name: 'bad name', url: 'https://example.com', update_interval_hours: 24 }), /不能包含空格/)
  assert.match(validateSubscription({ name: 'airport', url: 'ftp://example.com', update_interval_hours: 24 }), /http/)
  assert.match(validateSubscription({ name: 'airport', url: 'https://example.com', update_interval_hours: 0 }), /更新间隔/)
  assert.equal(validateNode({ tag: 'proxy-main', link: 'socks5h://10.0.0.4:2000#proxy-main' }), '')
  assert.equal(validateNode({ link: 'vless://uuid@example.com:443?security=tls#vless-node' }), '')
  assert.match(validateNode({ tag: 'provider/node', link: 'socks5h://10.0.0.4:2000' }), /不能包含/)
  assert.equal(validateNode({ editing: true, tag: 'existing', link: '' }), '')
  assert.match(validateNode({ link: 'not-a-link' }), /节点链接/)
  assert.match(validateNode({ link: 'vless://one\nvless://two' }), /一条节点链接/)
})

test('sing-box node display helpers preserve provider source and delay', () => {
  assert.equal(nodeSource('airport/Japan-01'), 'airport')
  assert.equal(nodeSource('proxy-main'), '手动节点')
  assert.equal(latestDelay({ delay: 42 }), '42 ms')
  assert.equal(latestDelay({}), '未测速')
  assert.equal(groupByName([{ name: 'proxy', nodes: [] }], 'proxy').name, 'proxy')
  assert.equal(groupByName([], 'proxy'), null)
})

test('sing-box API operations and navigation expose subscription and node management', () => {
  assert.equal(requestOperation('/api/v1/singbox/subscriptions'), 'singbox.read')
  assert.equal(requestOperation('/api/v1/singbox/subscriptions', 'POST'), 'singbox.manage')
  assert.equal(requestOperation('/api/v1/singbox/nodes'), 'singbox.read')
  assert.equal(requestOperation('/api/v1/singbox/select', 'POST'), 'singbox.manage')
  assert.equal(requestOperation('/api/v1/singbox/delay', 'POST'), 'singbox.manage')
  const read = (path) => readFileSync(new URL(path, import.meta.url), 'utf8')
  const page = read('../src/pages/SingBoxPage.vue')
  assert.match(page, /SingboxSubscriptions/)
  assert.match(page, /SingboxNodes/)
  const subscriptions = read('../src/components/singbox/SubscriptionsPanel.vue')
  assert.match(subscriptions, /新增订阅/)
  assert.match(subscriptions, /update_interval_hours/)
  assert.match(subscriptions, /\/api\/v1\/singbox\/subscriptions/)
  const nodes = read('../src/components/singbox/NodesPanel.vue')
  assert.match(nodes, /添加节点/)
  assert.match(nodes, /textarea id="singbox-node-link"/)
  assert.match(nodes, /props.manualNodes.find/)
  assert.match(nodes, /留空保持不变/)
  assert.doesNotMatch(nodes, /添加 SOCKS5 节点|服务器地址|server_port/)
  assert.match(nodes, /\/api\/v1\/singbox\/select/)
  assert.match(nodes, /\/api\/v1\/singbox\/delay/)
  assert.match(nodes, /应用出口/)
})
