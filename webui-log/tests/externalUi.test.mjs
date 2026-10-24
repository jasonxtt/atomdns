import test from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'

import { API_LEVEL, ARCHIVE_NAME, buildManifest, validVersion } from '../../scripts/external-ui-manifest.mjs'

const read = (path) => readFileSync(new URL(path, import.meta.url), 'utf8')

test('the manifest of an external package has the fields the host judges', () => {
  const manifest = buildManifest({ version: 'v1.2.3-4-gabc', now: new Date('2026-10-17T10:11:12.345Z') })
  assert.deepEqual(manifest, {
    manifest_schema: 1,
    ui_version: 'v1.2.3-4-gabc',
    built_at: '2026-10-17T10:11:12Z',
    api_level: API_LEVEL,
    requires_api_level: API_LEVEL
  })
  for (const bad of ['', '  ', 'a\nb', 'x'.repeat(65), undefined, 5]) {
    assert.equal(validVersion(bad), false, String(bad))
    assert.throws(() => buildManifest({ version: bad }))
  }
  assert.equal(ARCHIVE_NAME, 'rtdns-webui.tar.gz')
  assert.match(read('../../rust/native-host/src/update/mod.rs'), /pub\(crate\) const WEBUI_ASSET: &str = "rtdns-webui\.tar\.gz";/)
})

test('the build script and the host agree on the interface level', () => {
  const host = read('../../rust/native-host/src/ui_manifest.rs')
  assert.equal(Number(host.match(/pub\(crate\) const HOST_API_LEVEL: u64 = (\d+);/)[1]), API_LEVEL)
})

test('one build makes the WebUI package: relative URLs, a name-independent directory, a manifest and a tar.gz', () => {
  const config = read('../vite.config.js')
  assert.match(config, /base: '\.\/'/)
  assert.match(config, /path\.resolve\(__dirname, 'dist'\)/)
  const code = config.split('\n').filter((line) => !line.trim().startsWith('//')).join('\n')
  assert.doesNotMatch(code, /\/main\/|native-host\/ui/, 'nothing depends on a mount name or the program tree')
  const pkg = JSON.parse(read('../package.json'))
  assert.equal(pkg.scripts.build, 'node ../scripts/build-webui.mjs')
  assert.equal(pkg.scripts['build:package'], 'node ../scripts/build-webui.mjs --package')
  assert.equal(pkg.scripts['build:external'], undefined)
  const script = read('../../scripts/build-webui.mjs')
  assert.match(script, /ui-manifest\.json/)
  assert.match(script, /--format=ustar/)
  assert.match(script, /COPYFILE_DISABLE/)
  assert.doesNotMatch(script, /\bzip\b/)
})

test('the system page shows the WebUI version and links to every other mounted UI package', () => {
  const panel = read('../src/pages/system/SystemUpdateSection.vue')
  assert.match(panel, /entry\.path === '\/'/, 'ui/main is the WebUI, not an external package')
  assert.match(panel, /label="界面版本"/)
  assert.match(panel, /uiEntries\.value\.filter\(\(entry\) => !isWebui\(entry\)\)/)
  assert.match(panel, /health\.value\?\.external_ui/)
  assert.match(panel, /entry\.kind === 'mosdns_ui' && entry\.mounted/)
  assert.match(panel, /entry\.kind === 'mosdns_ui' && !entry\.mounted/)
  assert.match(panel, /v-for="entry in mountedPackages"/)
  assert.match(panel, /encodeURIComponent\(entry\.name\)/)
  assert.doesNotMatch(panel, /'\/main\/'|"\/main\/"/, 'no mount name is hard-coded')
  assert.match(panel, /new URL\('\.\/', window\.location\.href\)\.pathname/, 'a package does not link to itself')
})

test('a port change made in an external UI stays under that UI\'s own path', () => {
  const control = read('../src/pages/system/SystemServiceSection.vue')
  assert.match(control, /url\.pathname = new URL\('\.\/', window\.location\.href\)\.pathname/)
  assert.doesNotMatch(control, /\/main\//)
})

test('the program embeds no UI: the native build runs no frontend build and build.rs reads no UI files', () => {
  const build = read('../../scripts/build-rust-native.sh').split('\n').filter((line) => !line.startsWith('#')).join('\n')
  assert.doesNotMatch(build, /npm|log1/)
  const manifest = read('../../scripts/native-build-manifest.mjs')
  assert.doesNotMatch(manifest, /log1|coremain|native-host\/ui/)
  const buildRs = read('../../rust/native-host/build.rs')
  assert.doesNotMatch(buildRs, /log1|coremain|"ui"|include_bytes/)
  assert.doesNotMatch(read('../package.json'), /log1/)
})
