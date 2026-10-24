<script setup>
import FlagSwitches from '../../components/rules/FlagSwitches.vue'
import ListEditor from '../../components/rules/ListEditor.vue'
</script>

<template>
  <div class="rt-settings-page">
    <FlagSwitches page="mode" />
    <ListEditor tag="realiplist" title="!CN fakeip filter" hint="名单里的域名向国外 DNS 解析并返回真实 IP，不使用 FakeIP。适合必须用真实 IP 连接的域名。" />
    <ListEditor tag="cnfakeipfilter" title="CN fakeip filter" hint="名单里的域名向国内 DNS 解析并返回真实 IP，不使用 FakeIP。适合必须用真实 IP 连接的域名。" />
  </div>
</template>
