import test from 'node:test'
import assert from 'node:assert/strict'
import { loadCacheInventory, parseCacheMetrics, summarizeCacheFlush } from '../src/utils/cacheInventory.js'

test('catalog uses actual ordered tags including empty catalog', async () => {
  const caches = await loadCacheInventory(async () => ({ schema_version: 1, caches: [{ tag: 'zeta' }, { tag: 'alpha' }] }))
  assert.deepEqual(caches.map((cache) => cache.tag), ['zeta', 'alpha'])
  assert.deepEqual(await loadCacheInventory(async () => ({ schema_version: 1, caches: [] })), [])
})
test('a cache shows its configured label and falls back to the tag', async () => {
  const caches = await loadCacheInventory(async () => ({ schema_version: 1, caches: [{ tag: 'cache_cn', label: ' 国内查询缓存 ' }, { tag: 'plain' }, { tag: 'blank', label: '  ' }] }))
  assert.deepEqual(caches.map((cache) => [cache.tag, cache.name, cache.labelled]), [['cache_cn', '国内查询缓存', true], ['plain', 'plain', false], ['blank', 'blank', false]])
})
test('failures, an unsupported catalog and unknown/malformed schema are errors, never a fixed catalog', async () => {
  await assert.rejects(loadCacheInventory(async () => ({ supported: false })), /不一致/)
  for (const status of [400, 401, 403, 404, 500, 501]) {
    await assert.rejects(loadCacheInventory(async () => { throw Object.assign(new Error('failed'), { status }) }))
  }
  await assert.rejects(loadCacheInventory(async () => { throw new Error('timeout') }))
  for (const data of [{ schema_version: 2, caches: [] }, {}, { schema_version: 1, caches: [{ tag: '' }] }, { schema_version: 1, caches: [{ tag: 'a' }, { tag: 'a' }] }]) {
    await assert.rejects(loadCacheInventory(async () => data))
  }
})
test('metrics preserve missing values, exact tags, escaped labels and genuine zero', () => {
  assert.deepEqual(parseCacheMetrics('', 'missing'), { query_total: null, hit_total: null, lazy_hit_total: null, size_current: null })
  const text = 'mosdns_cache_query_total{tag="a"} 0\nmosdns_cache_hit_total{tag="a"} 2\nmosdns_cache_lazy_hit_total{tag="a"} NaN\nmosdns_cache_size_current{tag="a"} 7\nmosdns_cache_query_total{tag="aa"} 55'
  assert.deepEqual(parseCacheMetrics(text, 'a'), { query_total: 0, hit_total: 2, lazy_hit_total: null, size_current: 7 })
  assert.equal(parseCacheMetrics('mosdns_cache_size_current{tag="a\\"b\\\\c\\nd"} 9', 'a"b\\c\nd').size_current, 9)
})
test('partial batch failures retain specific tags and actual successful count', () => {
  assert.deepEqual(summarizeCacheFlush([{ tag: 'alpha' }, { tag: 'beta' }, { tag: 'gamma' }], [{ status: 'fulfilled' }, { status: 'rejected' }, { status: 'fulfilled' }]), { succeeded: 2, failed: ['beta'] })
})
