import test from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'

import { requestOperation } from '../src/api/runtimeCapabilities.js'
import { CACHE_RANGES, LEARNING_RANGES, fieldError, formatCount, formatDuration, splitDuration, toSeconds } from '../src/utils/tuning.js'

const read = (path) => readFileSync(new URL(path, import.meta.url), 'utf8')

test('a duration is split into the largest unit that divides it evenly', () => {
  assert.deepEqual(splitDuration(604800), { value: 7, unit: 'day' })
  assert.deepEqual(splitDuration(21600), { value: 6, unit: 'hour' })
  assert.deepEqual(splitDuration(36000), { value: 10, unit: 'hour' })
  assert.deepEqual(splitDuration(1800), { value: 30, unit: 'minute' })
  assert.deepEqual(splitDuration(90), { value: 90, unit: 'second' })
  assert.deepEqual(splitDuration(0), { value: 0, unit: 'day' })
  assert.deepEqual(splitDuration(259200000), { value: 3000, unit: 'day' })
  assert.deepEqual(splitDuration('x'), { value: 0, unit: 'day' })
})

test('an amount and a unit give whole seconds; an empty amount is not a number', () => {
  assert.equal(toSeconds(7, 'day'), 604800)
  assert.equal(toSeconds('6', 'hour'), 21600)
  assert.equal(toSeconds(1.5, 'day'), 129600)
  assert.equal(toSeconds(0.00001, 'second'), 0)
  assert.ok(Number.isNaN(toSeconds('', 'day')))
  assert.ok(Number.isNaN(toSeconds(undefined, 'day')))
})

test('durations are written in the unit that fits; very long ones in years', () => {
  assert.equal(formatDuration(604800), '7 天')
  assert.equal(formatDuration(21600), '6 小时')
  assert.equal(formatDuration(129600), '1.5 天')
  assert.equal(formatDuration(60), '1 分钟')
  assert.equal(formatDuration(30), '30 秒')
  assert.equal(formatDuration(259200000), '约 8.2 年')
  assert.equal(formatDuration(4294967295), '约 136.2 年')
  assert.equal(formatDuration(Number.NaN), '—')
  assert.equal(formatCount(20000000), '20,000,000')
})

test('a value outside the backend range, or not a whole number, is refused with a reason', () => {
  const [low, high] = CACHE_RANGES.size
  assert.equal(fieldError(low, CACHE_RANGES.size, '容量上限', 'count'), '')
  assert.equal(fieldError(high, CACHE_RANGES.size, '容量上限', 'count'), '')
  assert.match(fieldError(low - 1, CACHE_RANGES.size, '容量上限', 'count'), /1,000 到 100,000,000/)
  assert.match(fieldError(Number.NaN, CACHE_RANGES.size, '容量上限', 'count'), /需要是整数/)
  assert.match(fieldError(1.5, CACHE_RANGES.size, '容量上限', 'count'), /需要是整数/)
  assert.equal(fieldError(0, CACHE_RANGES.lazy_cache_ttl, '过期缓存保留', 'duration'), '')
  assert.match(fieldError(59, LEARNING_RANGES.ttl, '有效期', 'duration'), /1 分钟 到 约 10 年|1 分钟到/)
  assert.equal(fieldError(60, LEARNING_RANGES.ttl, '有效期', 'duration'), '')
})

test('the settings routes belong to their own operations', () => {
  assert.equal(requestOperation('/api/v1/cache/settings'), 'cache.settings')
  assert.equal(requestOperation('/api/v1/cache/settings/cache_all', 'POST'), 'cache.settings')
  assert.equal(requestOperation('/api/v1/learning/settings', 'DELETE'), 'learning.settings')
  assert.equal(requestOperation('/api/v1/cache/inventory'), 'cache.inventory')
})

test('the cache and learning pages edit through the settings routes and confirm a cache reset', () => {
  const cache = read('../src/components/rules/CacheSettingsPanel.vue')
  assert.match(cache, /\/api\/v1\/cache\/settings/)
  assert.match(cache, /supportsOperation\('cache\.settings'\)/)
  assert.match(cache, /openConfirm/, 'a change empties the cache, so it asks first')
  const learning = read('../src/components/rules/LearningData.vue')
  assert.match(learning, /\/api\/v1\/learning\/settings/)
  assert.match(learning, /supportsOperation\('learning\.settings'\)/)
  assert.doesNotMatch(learning, /openConfirm\([^)]*清空缓存/, 'learning settings never clear anything')
})

test('the learning detail shows the rule label and keeps the rule tag as its tooltip', () => {
  const learning = read('../src/components/rules/LearningData.vue')
  assert.match(learning, /item\.reason_label \|\| item\.reason/)
  assert.match(learning, /:title="item\.reason"/)
  const settings = read('../src/components/rules/CacheSettingsPanel.vue')
  assert.match(settings, /row\.name/)
  assert.doesNotMatch(settings, /\$\{row\.tag\}/, 'texts and aria labels use the display name')
})
