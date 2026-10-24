<script setup>
import FlagSwitches from '../../components/rules/FlagSwitches.vue'
import ListEditor from '../../components/rules/ListEditor.vue'
import OverridesPanel from '../../components/rules/OverridesPanel.vue'
import UpstreamPanel from '../../components/rules/UpstreamPanel.vue'
</script>

<template>
  <div class="rt-settings-page">
    <FlagSwitches page="upstream" />
    <UpstreamPanel />
    <OverridesPanel />
    <ListEditor tag="ddnslist" title="DDNS 域名" hint="名单里的域名按动态域名处理，适合经常变 IP 的域名；是否优先用国外上游解析，由上面的开关决定。" />
  </div>
</template>
