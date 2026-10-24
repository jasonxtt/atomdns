// Build-only Node helper. The resulting native executable does not use Node.
import { createHash } from 'node:crypto'
import { lstatSync, readdirSync, readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { execFileSync } from 'node:child_process'
const root = process.cwd()
const version = process.env.MOSDNS_BUILD_VERSION
if (!version?.trim() || /[\r\n]/.test(version)) throw new Error('invalid product version')
const [mode, manifestPath, artifactPath] = process.argv.slice(2)
const hash = p => createHash('sha256').update(readFileSync(path.resolve(root,p))).digest('hex')
if (mode === 'prepare') {
  const files = new Map()
  function collect(relative) {
    const stat = lstatSync(path.join(root,relative))
    if (stat.isSymbolicLink()) throw new Error(`symlink build input: ${relative}`)
    if (stat.isDirectory()) {
      for (const name of readdirSync(path.join(root,relative)).sort()) {
        if (['target','node_modules','.git'].includes(name)) continue
        collect(`${relative}/${name}`)
      }
    } else if (stat.isFile()) {
      files.set(relative,hash(relative))
    } else throw new Error(`non-regular build input: ${relative}`)
  }
  // The program is built from `rust/` alone (it embeds no WebUI).
  collect('rust')
  for (const file of ['scripts/build-rust-native.sh','scripts/native-build-manifest.mjs']) collect(file)
  const source=Object.fromEntries([...files].sort(([a],[b])=>a.localeCompare(b)))
  writeFileSync(manifestPath,JSON.stringify({schema_version:1,runtime:'rust',version,source_id:process.env.BUILD_SOURCE_ID||null,source,source_manifest_sha256:createHash('sha256').update(JSON.stringify(source)).digest('hex'),artifact:null},null,2)+'\n')
} else if (mode === 'finish') {
  const manifest=JSON.parse(readFileSync(manifestPath,'utf8'))
  for (const [file,digest] of Object.entries(manifest.source)) if (hash(file)!==digest) throw new Error(`input changed during build: ${file}`)
  const actual=execFileSync(artifactPath,['version'],{encoding:'utf8'})
  if (actual!==`${version}\n` || manifest.version!==version) throw new Error('native version mismatch')
  manifest.artifact={path:artifactPath,sha256:hash(artifactPath),version_stdout:actual}
  manifest.rustc=execFileSync('rustc',['-vV'],{encoding:'utf8'}).trim()
  writeFileSync(manifestPath,JSON.stringify(manifest,null,2)+'\n')
} else throw new Error('expected prepare or finish')
