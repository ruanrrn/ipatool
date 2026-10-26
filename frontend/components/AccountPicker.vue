<template>
  <!-- No accounts alert -->
  <div
    v-if="accounts.length === 0"
    class="account-alert account-alert--fused-bottom"
  >
    <div class="flex items-start gap-3">
      <SvgIcon
        class="w-4 h-4 flex-shrink-0 mt-0.5 text-warning"
        :icon="alertTriangleIcon"
      />
      <div class="flex-1">
        <h4 class="text-body font-semibold text-txt">
          需要先登录账号
        </h4>
        <p class="text-caption text-txt-secondary mt-1">
          请先在"设置"页登录 Apple ID 账号，然后才能搜索应用。
        </p>
        <MobileButton
          type="primary"
          size="small"
          class="mt-2"
          plain
          @click="handleGoToAccountTab"
        >
          前往登录
        </MobileButton>
      </div>
    </div>
  </div>

  <!-- Multiple accounts: show chip with picker -->
  <div
    v-else-if="accounts.length > 1"
    class="account-select-bar account-select-bar--fused-bottom"
  >
    <button
      v-if="selectedAccount !== null && selectedAccount !== undefined && selectedAccount !== '' && accounts[selectedAccount]"
      type="button"
      class="account-picker-chip"
      @click="showPicker = true"
    >
      <AccountChip
        :email="accounts[selectedAccount].email"
        :region="getRegionLabel(accounts[selectedAccount]?.region || 'US')"
        :show-arrow="true"
      />
    </button>
  </div>

  <!-- Single account: show static chip -->
  <div
    v-else-if="accounts.length === 1"
    class="account-select-bar account-select-bar--fused-bottom"
  >
    <AccountChip
      :email="accounts[0].email"
      :region="getRegionLabel(accounts[0]?.region || 'US')"
    />
  </div>

  <!-- Account Picker Bottom Sheet -->
  <Transition name="sheet-fade">
    <div
      v-if="showPicker"
      class="version-sheet-overlay"
      @click.self="showPicker = false"
    >
      <Transition name="sheet-slide">
        <div
          v-if="showPicker"
          class="version-sheet account-picker-sheet"
        >
          <div
            class="version-sheet__handle"
            @click="showPicker = false"
          />
          <div class="version-sheet__header">
            <div class="version-sheet__header-info">
              <h3 class="version-sheet__app-name">
                选择账号
              </h3>
              <p class="version-sheet__app-meta">
                切换下载与版本查询使用的 Apple ID
              </p>
            </div>
            <button
              class="version-sheet__close"
              @click="showPicker = false"
            >
              <SvgIcon
                class="w-5 h-5"
                :icon="closeIcon"
              />
            </button>
          </div>

          <div class="version-sheet__section">
            <div class="account-picker-list">
              <button
                v-for="(account, index) in accounts"
                :key="account.token || account.dsid || account.email || index"
                type="button"
                class="account-picker-item"
                :class="{ 'account-picker-item--active': selectedAccount === index }"
                @click="selectAccount(index)"
              >
                <div class="account-picker-item__main">
                  <span class="account-picker-item__email">{{ account.email }}</span>
                  <span class="account-picker-item__region">{{ getRegionLabel(account.region || 'US') }}</span>
                </div>
                <div class="account-picker-item__radio">
                  <div
                    v-if="selectedAccount === index"
                    class="account-picker-item__radio-fill"
                  />
                </div>
              </button>
            </div>
          </div>
        </div>
      </Transition>
    </div>
  </Transition>
</template>

<script setup>
import { ref, watch } from 'vue'
import MobileButton from './MobileButton.vue'
import AccountChip from './AccountChip.vue'
import SvgIcon from './SvgIcon.vue'
import { formatRegion } from '../utils/region.js'
import { STORAGE_KEYS } from '../utils/storage.js'
import { accountIdentityKey } from '../composables/useAccounts.js'
import alertTriangleIcon from '../assets/icons/alert-triangle.svg?raw'
import closeIcon from '../assets/icons/close.svg?raw'

const props = defineProps({
  accounts: {
    type: Array,
    default: () => []
  },
  selectedAccount: {
    type: Number,
    default: null
  }
})

const emit = defineEmits(['update:selectedAccount', 'account-change', 'go-to-account'])

const showPicker = ref(false)

// Get region label
const getRegionLabel = (region) => {
  return formatRegion(region)
}

// Save selected account identity to localStorage
const saveSelectedAccountIdentity = () => {
  const account = props.accounts[props.selectedAccount]
  const key = accountIdentityKey(account)
  if (key) {
    localStorage.setItem(STORAGE_KEYS.SELECTED_ACCOUNT_KEY, key)
  }
}

// Select account from picker
const selectAccount = async (index) => {
  showPicker.value = false
  emit('update:selectedAccount', index)
  emit('account-change', index)
  saveSelectedAccountIdentity()
}

// Handle go to account tab
const handleGoToAccountTab = () => {
  emit('go-to-account')
}

// Normalize account index
const normalizeAccountIndex = (value) => {
  if (value === null || value === undefined || value === '') return null
  const parsed = Number.parseInt(String(value), 10)
  return Number.isInteger(parsed) && parsed >= 0 ? parsed : null
}

// Watch for selectedAccount changes and save to localStorage
watch(() => props.selectedAccount, (newValue) => {
  const normalizedIndex = normalizeAccountIndex(newValue)
  if (normalizedIndex !== null) {
    localStorage.setItem(STORAGE_KEYS.SELECTED_ACCOUNT_INDEX, String(normalizedIndex))
  }
})

// Restore selected account from localStorage
const restoreSelectedAccount = () => {
  const savedAccountKey = localStorage.getItem(STORAGE_KEYS.SELECTED_ACCOUNT_KEY)
  if (savedAccountKey) {
    const matchedIndex = props.accounts.findIndex(account => accountIdentityKey(account) === savedAccountKey)
    if (matchedIndex >= 0) {
      emit('update:selectedAccount', matchedIndex)
      return true
    }
  }

  const savedAccountIndex = localStorage.getItem(STORAGE_KEYS.SELECTED_ACCOUNT_INDEX)
  if (savedAccountIndex !== null && savedAccountIndex !== '' && !isNaN(parseInt(savedAccountIndex, 10)) && parseInt(savedAccountIndex, 10) < props.accounts.length) {
    const index = parseInt(savedAccountIndex, 10)
    emit('update:selectedAccount', index)
    saveSelectedAccountIdentity()
    return true
  }

  return false
}

// Auto-select first account if none selected
const autoSelectFirstAccount = () => {
  if (props.accounts.length > 0 && (props.selectedAccount === null || props.selectedAccount === undefined || props.selectedAccount === '')) {
    if (!restoreSelectedAccount()) {
      emit('update:selectedAccount', 0)
      saveSelectedAccountIdentity()
    }
  }
}

// Expose helper functions
defineExpose({
  autoSelectFirstAccount,
  saveSelectedAccountIdentity,
  restoreSelectedAccount
})
</script>

<style scoped>
/* Account Alert */
.account-alert {
  padding: 12px 14px;
  margin-bottom: 12px;
  background: var(--color-surface-soft);
  border: 1px solid var(--color-border-warning);
  border-radius: 14px;
}

.account-alert--fused-bottom {
  border-radius: 0 0 14px 14px;
  margin-top: -1px;
}

/* Account Select Bar */
.account-select-bar--fused-bottom {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 10px 12px;
  margin-bottom: 12px;
  margin-top: -1px;
  background: var(--color-bg-surface);
  border: 1px solid var(--color-border);
  border-top: none;
  border-radius: 0 0 14px 14px;
}

.account-picker-chip {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  min-width: 0;
  padding: 0;
  border: none;
  background: transparent;
  cursor: pointer;
  text-align: left;
}

.account-picker-chip :deep(.account-chip) {
  flex: 1;
  min-width: 0;
}

.account-picker-chip__chevron {
  width: 18px;
  height: 18px;
  flex-shrink: 0;
  color: var(--color-text-muted);
}

/* Account Picker Sheet */
.account-picker-sheet {
  max-height: min(70vh, 560px);
}

.account-picker-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.account-picker-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  width: 100%;
  padding: 14px 12px;
  border-radius: 12px;
  border: 1px solid var(--color-border);
  background: var(--color-surface-muted);
  text-align: left;
}

.account-picker-item--active {
  background: var(--color-success-soft);
  border-color: var(--color-success-border);
}

.account-picker-item__main {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
}

.account-picker-item__email {
  font-size: 14px;
  font-weight: 600;
  color: var(--color-text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.account-picker-item__region {
  font-size: 12px;
  color: var(--color-text-muted);
}

.account-picker-item__radio {
  width: 20px;
  height: 20px;
  border-radius: 999px;
  border: 2px solid var(--color-border-divider);
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}

.account-picker-item__radio-fill {
  width: 10px;
  height: 10px;
  border-radius: 999px;
  background: var(--color-primary);
}

/* Version Sheet Overlay */
.version-sheet-overlay {
  position: fixed;
  inset: 0;
  z-index: 1000;
  background: rgba(var(--rgb-black), 0.5);
  display: flex;
  align-items: flex-end;
  justify-content: center;
}

.version-sheet {
  width: 100%;
  max-width: 600px;
  background: var(--color-bg-surface);
  border-radius: 20px 20px 0 0;
  max-height: min(85vh, 700px);
  overflow: hidden;
  display: flex;
  flex-direction: column;
}

.version-sheet__handle {
  width: 36px;
  height: 4px;
  background: var(--color-border-divider);
  border-radius: 2px;
  margin: 12px auto 8px;
  flex-shrink: 0;
  cursor: pointer;
}

.version-sheet__header {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 8px 16px 16px;
  flex-shrink: 0;
}

.version-sheet__header-info {
  flex: 1;
  min-width: 0;
}

.version-sheet__app-name {
  font-size: 18px;
  font-weight: 600;
  color: var(--color-text);
  margin: 0 0 2px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.version-sheet__app-meta {
  font-size: 13px;
  color: var(--color-text-muted);
  margin: 0;
}

.version-sheet__close {
  width: 32px;
  height: 32px;
  border-radius: 50%;
  border: none;
  background: var(--color-surface-muted);
  color: var(--color-text);
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  transition: background-color 0.2s;
}

.version-sheet__close:hover {
  background: var(--color-surface);
}

.version-sheet__section {
  padding: 16px;
  overflow-y: auto;
  flex: 1;
}

/* Dark mode */
.dark .account-picker-item {
  background: var(--color-surface);
  border-color: var(--color-surface-muted);
}

.dark .account-picker-item--active {
  background: var(--color-success-soft);
  border-color: var(--color-success-border);
}

.dark .account-picker-item__email {
  color: var(--color-text);
}

.dark .account-picker-item__region {
  color: var(--color-text-muted);
}

.dark .account-picker-item__radio {
  border-color: var(--color-border);
}

.dark .version-sheet__app-name {
  color: var(--color-text);
}

.dark .version-sheet__app-meta {
  color: var(--color-text-muted);
}

.dark .version-sheet__close {
  background: var(--color-surface-muted);
  color: var(--color-text);
}

/* Transitions */
.sheet-fade-enter-active,
.sheet-fade-leave-active {
  transition: opacity 0.3s ease;
}

.sheet-fade-enter-from,
.sheet-fade-leave-to {
  opacity: 0;
}

.sheet-slide-enter-active,
.sheet-slide-leave-active {
  transition: transform 0.3s cubic-bezier(0.32, 0.72, 0, 1);
}

.sheet-slide-enter-from,
.sheet-slide-leave-to {
  transform: translateY(100%);
}

/* Responsive */
@media (min-width: 640px) {
  .account-picker-item {
    padding: 12px 16px;
  }

  .version-sheet {
    border-radius: 20px;
    margin: 16px;
    max-height: min(80vh, 600px);
  }

  .account-picker-item {
    padding: 16px;
  }
}
</style>
