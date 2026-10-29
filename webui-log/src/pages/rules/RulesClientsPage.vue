<script setup>
import ClientAliasPanel from '../../components/rules/ClientAliasPanel.vue'
import FlagSwitches from '../../components/rules/FlagSwitches.vue'
import ListEditor from '../../components/rules/ListEditor.vue'
</script>

<template>
  <div class="rt-settings-page">
    <section class="panel rt-panel-group">
      <FlagSwitches page="clients" />
      <div class="rt-divider" role="separator"><i></i></div>
      <ListEditor tag="client_ip" title="客户端 IP" hint="配合上面的“客户端分流”使用：rtdns 要直接作为 DNS 下发给客户端（占用 53 端口），名单才生效。例如 192.0.2.10 或 198.51.100.0/24。" />
    </section>
    <ClientAliasPanel />
  </div>
</template>
