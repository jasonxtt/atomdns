import test from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { getJSON, getJSONWithHeaders } from '../src/api/http.js'
import { parseDistinctDomainCount } from '../src/utils/overviewMetrics.js'

test('exact domain counts are not limited to the rank size and missing metadata is unknown', () => {
  for (const count of [0, 1, 499, 500, 550, 100000, 400000]) {
    assert.equal(parseDistinctDomainCount(String(count)), count)
  }
  for (const value of [null, undefined, '', ' ', '-1', '2.5', '500+', 'NaN', '9007199254740992']) {
    assert.equal(parseDistinctDomainCount(value), null)
  }
})

test('JSON metadata requests preserve the array and header; ordinary JSON stays compatible', async () => {
  const original = globalThis.fetch
  globalThis.fetch = async () => Response.json([{ key: 'one.example', count: 3 }], {
    headers: { 'X-Audit-Distinct-Domains': '550' }
  })
  try {
    const result = await getJSONWithHeaders('/test-metadata')
    assert.deepEqual(result.data, [{ key: 'one.example', count: 3 }])
    assert.equal(parseDistinctDomainCount(result.headers.get('x-audit-distinct-domains')), 550)
    assert.deepEqual(await getJSON('/test-metadata'), result.data)
    globalThis.fetch = async () => Response.json({ error: 'audit read capacity exhausted' }, { status: 503 })
    await assert.rejects(() => getJSONWithHeaders('/test-metadata'), error => error.status === 503)
  } finally {
    globalThis.fetch = original
  }
})

test('the overview reads exact domain metadata instead of counting rank rows', () => {
  const source = readFileSync(new URL('../src/pages/OverviewPage.vue', import.meta.url), 'utf8')
  assert.match(source, /getJSONWithHeaders\(`/)
  assert.match(source, /parseDistinctDomainCount\(/)
  assert.doesNotMatch(source, /domainCount\.value\s*=.*domains\.length/)
  assert.doesNotMatch(source, /distinctCountText\(domainCount\.value\)/)
})
