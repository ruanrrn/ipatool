<template>
  <p class="section-label text-txt-secondary dark:text-txt-dark-secondary">
    Apple ID 账号
  </p>
  <div class="settings-card">
    <div
      v-for="(account, index) in accounts"
      :key="getAccountKey(account, index)"
      class="settings-row settings-row--account"
    >
      <div class="sr-left">
        <div class="sr-icon sr-icon--apple">
          🍎
        </div>
        <div class="sr-label">
          {{ account.email }}
          <span
            v-if="account.loggedInAt"
            class="sr-login-time"
          >
            {{ getLoginLabel(account.loggedInAt) }}
          </span>
        </div>
      </div>
      <div class="sr-right">
        <span>{{ getRegionLabel(account.region || 'US') }}</span>
        <button
          class="sr-btn sr-btn--refresh"
          title="重新登录"
          aria-label="重新登录"
          @click.stop="emit('relogin-account', account)"
        >
          ↻
        </button>
        <button
          class="sr-btn sr-btn--delete"
          @click.stop="emit('delete-account', account)"
        >
          ✕
        </button>
      </div>
    </div>

    <button
      class="settings-row settings-row--interactive"
      @click="emit('navigate-to-account')"
    >
      <div class="sr-left">
        <div class="sr-icon sr-icon--add">
          +
        </div>
        <div class="sr-label sr-label--brand">
          添加账号
        </div>
      </div>
      <div class="sr-right">
        <span class="sr-arrow">›</span>
      </div>
    </button>
  </div>
</template>

<script setup>
const emit = defineEmits(['delete-account', 'navigate-to-account', 'relogin-account'])

defineProps({
  accounts: {
    type: Array,
    default: () => []
  },
  getAccountKey: {
    type: Function,
    required: true
  },
  getLoginLabel: {
    type: Function,
    required: true
  },
  getRegionLabel: {
    type: Function,
    required: true
  }
})
</script>
