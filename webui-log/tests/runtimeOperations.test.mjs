import test from 'node:test'
import assert from 'node:assert/strict'
import { validateRuntimeCapabilities, createRuntimeCapabilityClient } from '../src/api/runtimeCapabilities.js'
const old = {schema_version:1, runtime:'rust',special_groups:{enabled:false},upstream_protocols:[],rule_formats:[],unsupported_features:[],endpoints:{audit_v1:true,audit_v2:true,cache_inventory_get:true,metrics_get:true,manual_rules:{show_get:true,save_get:true,post:false},special_groups:{get:true,post:false,delete:false},upstream:{tags_get:true,config_get:true,runtime_get:true,config_post:false},diversion_sources:{list_get:false,put:false,delete:false}}}
test('a missing or incomplete operation matrix is invalid discovery',async()=>{
 assert.throws(()=>validateRuntimeCapabilities({...old,ui_operations:{}}),/invalid runtime/)
 assert.throws(()=>validateRuntimeCapabilities(old),/invalid runtime/)
 await assert.rejects(createRuntimeCapabilityClient(async()=>old)(),/invalid runtime/)
})

const runtime = await import('../src/api/runtimeCapabilities.js')
const {OPERATION_IDS,capabilityState,capabilityFetch,requestOperation,refreshRuntimeCapabilities,supportsOperation} = runtime
const matrix = (supported = []) => Object.fromEntries(OPERATION_IDS.map(id=>[id,{supported:supported.includes(id),reason:supported.includes(id)?null:'未支持'}]))
const native = (supported = []) => ({...old,ui_operations:matrix(supported)})
const install = value => {capabilityState.status='ready';capabilityState.value={kind:'native',...value};capabilityState.error='';capabilityState.cacheTags=new Set()}
test('every false family suppresses actual fetch for reads, refreshes and stale writes',async()=>{
 install(native())
 let calls=0;const original=globalThis.fetch;globalThis.fetch=async()=>{calls++;throw new Error('network reached')}
 const paths=[
 ['system.health','/api/v1/system/health'],['audit.read','/api/v2/audit/logs'],['audit.control','/api/v1/audit/start'],['audit.control','/api/v1/audit/stop'],['audit.control','/api/v1/audit/clear'],['audit.capacity','/api/v1/audit/capacity'],['query.rank','/api/v2/audit/rank/domain'],['cache.inventory','/api/v1/cache/inventory'],['cache.manage','/plugins/cache_one/flush'],['metrics.cache','/metrics'],['rules.local.read','/plugins/special_manual_50/show'],['rules.local.manage','/plugins/special_manual_50/post'],['groups.read','/api/v1/special-groups'],['groups.manage','/api/v1/special-groups/50','DELETE'],['groups.upstreams','/api/v1/upstream/sources'],['upstreams.read','/api/v1/upstream/tags'],['upstreams.manage','/api/v1/upstream/config','POST'],['rules.diversion','/plugins/diversion_50/config'],['rules.adguard','/plugins/adguard/rules'],['rules.adguard','/plugins/adguard/update/1','POST'],['capture.logs','/api/v1/capture/start','POST'],['capture.logs','/api/v1/capture/logs'],['client.aliases','/plugins/clientname'],['flags.manage','/api/v1/flags'],['flags.manage','/api/v1/flags/lazy_cache','POST'],['flags.manage','/api/v1/flags/lazy_cache','DELETE'],['cache.requery','/plugins/requery/trigger','POST'],['cache.requery','/plugins/requery/scheduler/config','POST'],['lists.remembered','/plugins/my_fakeiplist/show'],['lists.remembered','/plugins/top_domains/save'],['lists.remembered','/plugins/route_learner/status'],['lists.remembered','/plugins/route_learner/entries'],['system.restart','/api/v1/system/restart','POST'],['system.webui_port','/api/v1/system/webui-port'],['system.config_management','/api/v1/config/export','POST'],['system.update','/api/v1/update/status'],['system.domain_generation','/api/v1/domain-generation'],['system.global_overrides','/api/v1/overrides']]
 try{for(const [id,url,method='GET'] of paths){assert.equal(requestOperation(url,method),id,url);await assert.rejects(()=>capabilityFetch(url,{method}),e=>e.capabilityDisabled===true)}}finally{globalThis.fetch=original}
 assert.equal(calls,0)
})
test('new matrix requires all entries and valid bool/reason, allows additive unknown keys',()=>{
 for(const id of OPERATION_IDS){const value=native();delete value.ui_operations[id];assert.throws(()=>validateRuntimeCapabilities(value),/invalid runtime/)}
 for(const entry of [{supported:'true',reason:null},{supported:true,reason:'x'},{supported:false,reason:null},{supported:false,reason:' '}])assert.throws(()=>validateRuntimeCapabilities({...old,ui_operations:{...matrix(), 'system.health':entry}}),/invalid runtime/)
 assert.equal(validateRuntimeCapabilities({...native(),ui_operations:{...matrix(),future:{}}}).runtime,'rust')
})
test('the matrix names flags.manage and no longer the removed switch and appearance families',()=>{
 assert.equal(OPERATION_IDS.includes('flags.manage'),true)
 for(const id of ['switches.manage','appearance.server'])assert.equal(OPERATION_IDS.includes(id),false,id)
 assert.equal(requestOperation('/api/v1/appearance/text-color'),null,'the appearance routes are gone')
 assert.equal(new Set(OPERATION_IDS).size,OPERATION_IDS.length)
})
test('a 404 discovery is an error, never a Go fallback',async()=>{
 const client=createRuntimeCapabilityClient(async()=>{throw Object.assign(new Error('HTTP 404'),{status:404})})
 await assert.rejects(client(),/HTTP 404/)
})
test('discovery errors remain retryable without optional network requests',async()=>{
 let attempts=0;const client=createRuntimeCapabilityClient(async()=>{attempts++;if(attempts===1)throw Object.assign(new Error('unauthorized'),{status:401});return native()})
 await assert.rejects(client(),/unauthorized/);assert.equal((await client()).kind,'native');assert.equal(attempts,2)
 install(native());const original=globalThis.fetch;const calls=[]
 globalThis.fetch=async url=>{calls.push(url);if(calls.length===1)throw new Error('offline');return Response.json(native(['audit.read']))}
 try{
  await assert.rejects(refreshRuntimeCapabilities(),/offline/);assert.equal(capabilityState.status,'error')
  await assert.rejects(()=>capabilityFetch('/plugins/requery/trigger',{method:'POST'}),/offline/);assert.equal(calls.length,1)
  await refreshRuntimeCapabilities();assert.equal(supportsOperation('audit.read'),true);assert.equal(calls.length,2)
 }finally{globalThis.fetch=original}
})
test('successful inventory mutations reload capabilities and named cache tags retain own gate',async()=>{
 install(native(['groups.manage','cache.inventory','cache.manage']))
 const original=globalThis.fetch;const calls=[]
 globalThis.fetch=async url=>{calls.push(url);if(url==='/api/v1/capabilities')return Response.json(native(['groups.manage','rules.local.manage','cache.inventory','cache.manage']));if(url==='/api/v1/cache/inventory')return Response.json({schema_version:1,caches:[{tag:'hot_answers'}]});return Response.json({ok:true})}
 try{
  await capabilityFetch('/api/v1/special-groups',{method:'POST',body:'{}'});assert.equal(supportsOperation('rules.local.manage'),true);assert.deepEqual(calls.slice(0,2),['/api/v1/special-groups','/api/v1/capabilities'])
  await capabilityFetch('/api/v1/cache/inventory');assert.equal(requestOperation('/plugins/hot_answers/save'),'cache.manage');await capabilityFetch('/plugins/hot_answers/save')
 }finally{globalThis.fetch=original}
})
test('a disabled follow-up operation never reaches fetch',async()=>{
 install(native(['audit.read']))
 let calls=0;const original=globalThis.fetch;globalThis.fetch=async()=>{calls++;return Response.json({})}
 try{await assert.rejects(()=>capabilityFetch('/plugins/requery/trigger',{method:'POST'}),e=>e.capabilityDisabled);await assert.rejects(()=>capabilityFetch('/api/v1/flags'),e=>e.capabilityDisabled);await capabilityFetch('/api/v2/audit/logs');assert.equal(calls,1)}finally{globalThis.fetch=original}
})
test('accepted mutation ACK survives discovery refresh failure and existing view stays mounted but disabled',async()=>{
 install(native(['groups.manage']))
 const original=globalThis.fetch
 globalThis.fetch=async url=>{if(url==='/api/v1/capabilities')throw new Error('discovery temporarily failed');return Response.json({accepted:true})}
 try{
  const response=await capabilityFetch('/api/v1/special-groups',{method:'POST',body:'{}'})
  assert.equal(response.ok,true);assert.equal((await response.json()).accepted,true)
  assert.equal(capabilityState.status,'error');assert.equal(capabilityState.value.kind,'native');assert.equal(supportsOperation('groups.manage'),false)
  await assert.rejects(()=>capabilityFetch('/api/v1/special-groups',{method:'POST'}),/discovery temporarily failed/)
 }finally{globalThis.fetch=original}
})
test('unknown additive kind cannot override internal native classification',async()=>{
 const value=await createRuntimeCapabilityClient(async()=>({...native(),kind:'legacy'}))()
 assert.equal(value.kind,'native');assert.equal(value.ui_operations['system.restart'].supported,false)
})
test('cache inventory failures never invent a fixed cache menu',async()=>{
 const {loadCacheInventory}=await import('../src/utils/cacheInventory.js')
 await assert.rejects(()=>loadCacheInventory(async()=>{throw Object.assign(new Error('404'),{status:404})}),/404/)
 await assert.rejects(()=>loadCacheInventory(async()=>({supported:false})),/不一致/)
})
