// Builds the WebUI package: webui-log/dist/{index.html, app.js, app.css, chunks/, assets/, ui-manifest.json}.
// Installed as `<config root>/ui/main/` it is served at `/`; as any other `ui/<name>/` at `/<name>/`.
//   npm run build            the default WebUI directory only (webui-log/dist/)
//   npm run build:lg         the optional glass WebUI directory only (webui-log/dist-lg/)
//   npm run build:package    builds both default and glass UIs into release/atomdns-webui.tar.gz
//                            (default UI at top level + glass UI under bundled-ui/lg/)
// UI_VERSION overrides the version (default: git describe, then "dev"). The archive needs `tar`.
import { execFileSync } from 'node:child_process'
import { cpSync, existsSync, mkdirSync, mkdtempSync, readdirSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { ARCHIVE_NAME, BUNDLED_GLASS_SUBDIR, buildManifest, validVersion } from './external-ui-manifest.mjs'

const repository = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const workspace = path.join(repository, 'webui-log')
const glassOnly = process.env.VITE_UI_VARIANT === 'glass'

function describe() {
  try {
    return execFileSync('git', ['describe', '--tags', '--match', 'v*', '--always', '--dirty'], { cwd: repository, encoding: 'utf8' }).trim()
  } catch {
    return 'dev'
  }
}

const version = process.env.UI_VERSION || describe()
if (!validVersion(version)) throw new Error(`invalid UI version: ${JSON.stringify(version)}`)
const now = new Date()
const manifestText = `${JSON.stringify(buildManifest({ version, now }), null, 2)}\n`

function buildVariant(variant) {
  const isGlass = variant === 'glass'
  const outDir = path.join(workspace, isGlass ? 'dist-lg' : 'dist')
  const env = { ...process.env, UI_VERSION: version }
  if (isGlass) {
    env.VITE_UI_VARIANT = 'glass'
  } else {
    delete env.VITE_UI_VARIANT
  }
  execFileSync('npx', ['vite', 'build'], { cwd: workspace, stdio: 'inherit', env })
  if (!existsSync(path.join(outDir, 'index.html'))) {
    throw new Error(`the ${variant} build produced no index.html`)
  }
  writeFileSync(path.join(outDir, 'ui-manifest.json'), manifestText)
  console.log(`WebUI (${variant}) ${version}: ${outDir}`)
  return outDir
}

const primaryOut = buildVariant(glassOnly ? 'glass' : 'default')

if (process.argv.includes('--package')) {
  if (glassOnly) {
    throw new Error('use `npm run build:package` (without VITE_UI_VARIANT=glass) to build the release archive')
  }
  const glassOut = buildVariant('glass')
  const releaseDirectory = path.join(repository, 'release')
  mkdirSync(releaseDirectory, { recursive: true })
  const archive = path.join(releaseDirectory, ARCHIVE_NAME)
  rmSync(archive, { force: true })

  const stage = mkdtempSync(path.join(tmpdir(), 'atomdns-webui-pkg-'))
  try {
    cpSync(primaryOut, stage, { recursive: true })
    const bundledGlass = path.join(stage, BUNDLED_GLASS_SUBDIR)
    mkdirSync(path.dirname(bundledGlass), { recursive: true })
    cpSync(glassOut, bundledGlass, { recursive: true })

    // ustar, numeric root ownership, sorted names, no `./` prefix and (on macOS) no AppleDouble
    // files: the updater accepts plain files and directories only.
    const gnu = execFileSync('tar', ['--version'], { encoding: 'utf8' }).includes('GNU tar')
    const entries = readdirSync(stage).sort()
    const args = gnu
      ? ['--format=ustar', '--owner=0', '--group=0', '--numeric-owner', '--sort=name', '-czf', archive, '-C', stage, ...entries]
      : ['--format', 'ustar', '--uid', '0', '--gid', '0', '--numeric-owner', '-czf', archive, '-C', stage, ...entries]
    execFileSync('tar', args, { stdio: 'inherit', env: { ...process.env, COPYFILE_DISABLE: '1' } })
  } finally {
    rmSync(stage, { recursive: true, force: true })
  }
  console.log(`archive: ${archive}`)
}
