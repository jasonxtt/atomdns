<script setup>
import FlagSwitches from '../../components/rules/FlagSwitches.vue'
import ListEditor from '../../components/rules/ListEditor.vue'
import SubscriptionTable from '../../components/rules/SubscriptionTable.vue'
</script>

<template>
  <div class="rt-settings-page">
    <FlagSwitches page="blocking" tiles />
    <ListEditor tag="blocklist" title="黑名单" hint="名单里的域名会被屏蔽（需要上面的“黑名单屏蔽”开启）。支持 full:、domain:、keyword:、regexp: 等写法。" />
    <SubscriptionTable kind="adguard" title="AdGuard 订阅" hint="AdGuard 格式的广告拦截规则，需要“广告屏蔽”开启后已启用的订阅才生效。" />
  </div>
</template>
