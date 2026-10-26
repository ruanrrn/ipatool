<template>
  <div class="settings-page">
    <div class="settings-page__fixed px-5">
      <h1 class="page-title text-txt dark:text-txt-dark">
        设置
      </h1>
    </div>

    <div class="settings-page__scroll">
      <div class="settings-page__scroll-inner px-5">
        <SettingsAppleAccountsSection
          :accounts="accounts"
          :get-account-key="getAccountKey"
          :get-login-label="getLoginLabel"
          :get-region-label="getRegionLabel"
          @delete-account="handleDeleteAccount"
          @navigate-to-account="emit('navigate-to-account')"
          @relogin-account="(account) => emit('navigate-to-account', account.email)"
        />

        <SettingsGithubTokenSection
          v-model="githubTokenInput"
          :configured="githubTokenConfigured"
          :deleting="githubTokenDeleting"
          :masked-token="githubTokenMasked"
          :saving="githubTokenSaving"
          :updated-at="githubTokenUpdatedAt"
          @delete="handleDeleteGithubToken"
          @save="handleSaveGithubToken"
        />

        <SettingsAppearanceSection @navigate-to-appearance="emit('navigate-to-appearance')" />

        <SettingsWebDAVSection />

        <SettingsSecuritySection
          @logout="confirmLogout"
          @navigate-to-changepassword="emit('navigate-to-changepassword')"
        />

        <SettingsAboutSection
          :app-version="appVersion"
          :build-id="buildId"
        />
      </div>
    </div>
  </div>
</template>

<script setup>
/* global __APP_VERSION__, __APP_BUILD_ID__ */
import { computed, onMounted, ref } from 'vue'
import { useAppStore } from '../stores/app'
import { useAccounts, accountIdentityKey } from '../composables/useAccounts'
import { formatRegion } from '../utils/region.js'
import { API_BASE } from '../config.js'
import { apiFetch } from '../utils/api.js'
import { Confirm } from './MobileConfirm.vue'
import { Toast } from './MobileToast.vue'
import SettingsAboutSection from './SettingsAboutSection.vue'
import SettingsAppleAccountsSection from './SettingsAppleAccountsSection.vue'
import SettingsAppearanceSection from './SettingsAppearanceSection.vue'
import SettingsGithubTokenSection from './SettingsGithubTokenSection.vue'
import SettingsWebDAVSection from './SettingsWebDAVSection.vue'
import SettingsSecuritySection from './SettingsSecuritySection.vue'

const emit = defineEmits(['logout', 'navigate-to-appearance', 'navigate-to-account', 'navigate-to-changepassword'])
const appStore = useAppStore()
const { accounts, loadAccounts } = useAccounts()
const appVersion = __APP_VERSION__
const buildId = __APP_BUILD_ID__

const githubTokenInput = ref('')
const githubTokenSaving = ref(false)
const githubTokenDeleting = ref(false)

const githubTokenConfigured = computed(() => appStore.githubTokenStatus.configured)
const githubTokenMasked = computed(() => appStore.githubTokenStatus.maskedToken)
const githubTokenUpdatedAt = computed(() => {
  const value = appStore.githubTokenStatus.updatedAt
  if (!value) return ''
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) return value
  return date.toLocaleString()
})

const getAccountKey = (account, fallbackIndex = '') => accountIdentityKey(account) || account?.email || account?.token || account?.dsid || `account-${fallbackIndex}`

const getRegionLabel = (region) => formatRegion(region)

function getLoginLabel(loggedInAt) {
  if (!loggedInAt) return ''
  const secs = Math.max(0, Math.floor(Date.now() / 1000) - loggedInAt)
  if (secs < 60) return '刚刚登录'
  if (secs < 3600) return `登录于 ${Math.floor(secs / 60)} 分钟前`
  if (secs < 86400) return `登录于 ${Math.floor(secs / 3600)} 小时前`
  return `登录于 ${Math.floor(secs / 86400)} 天前`
}

async function handleSaveGithubToken() {
  const token = githubTokenInput.value.trim()
  if (!token) return
  githubTokenSaving.value = true
  try {
    await appStore.saveGithubToken(token)
    githubTokenInput.value = ''
    Toast.success('GitHub PAT 已保存')
  } catch (error) {
    Toast.error(error.message || '保存 GitHub PAT 失败')
  } finally {
    githubTokenSaving.value = false
  }
}

async function handleDeleteGithubToken() {
  const confirmed = await Confirm.show({
    title: '确认删除 GitHub PAT？',
    message: `将删除当前用于社区归档贡献的 GitHub PAT${githubTokenMasked.value ? `（${githubTokenMasked.value}）` : ''}。删除后将无法发布贡献，直到重新配置。`
  })
  if (!confirmed) return

  githubTokenDeleting.value = true
  try {
    await appStore.deleteGithubToken()
    githubTokenInput.value = ''
    Toast.success('GitHub PAT 已删除')
  } catch (error) {
    Toast.error(error.message || '删除 GitHub PAT 失败')
  } finally {
    githubTokenDeleting.value = false
  }
}

onMounted(() => {
  loadAccounts().catch((error) => {
    console.warn('Failed to load accounts:', error)
  })

  appStore.loadGithubTokenStatus().catch((error) => {
    console.warn('Failed to load GitHub token status:', error)
  })
})

async function confirmLogout() {
  const confirmed = await Confirm.show({
    title: '确认退出登录？',
    message: '退出后需要重新输入用户名和密码登录。已下载的 IPA 文件不会删除。'
  })
  if (!confirmed) return

  await appStore.logoutAdmin()
  Toast.success('已退出登录')
  emit('logout', {
    confirm: false,
    performLogout: false,
    toast: false
  })
}

async function handleDeleteAccount(account) {
  if (!account?.token) return
  const confirmed = await Confirm.show({
    title: '确认删除账号？',
    message: `将删除账号 ${account.email || ''}，该账号正在进行的任务可能会受影响。`
  })
  if (!confirmed) return

  try {
    const { response, data } = await apiFetch(`${API_BASE}/accounts/${account.token}`, {
      method: 'DELETE'
    })
    if (response.ok) {
      Toast.success(`${account.email} 已删除`)
      await loadAccounts()
    } else {
      Toast.error(data?.error || '删除失败')
    }
  } catch {
    Toast.error('删除请求失败')
  }
}
</script>

<style>
/* Page title */
.page-title {
  font-size: var(--font-size-title);
  font-weight: 700;
  line-height: 1.3;
  margin-bottom: var(--space-4);
  color: var(--color-text);
}
.dark .page-title {
  color: var(--color-text);
}

.settings-page {
  flex: 1;
  min-height: 0;
  overflow: hidden;
  display: flex;
  flex-direction: column;
  font-size: var(--font-size-md);
}

.settings-page__fixed {
  flex-shrink: 0;
  padding-top: max(var(--space-5), env(safe-area-inset-top));
  background: var(--color-bg);
}

.settings-page__scroll {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
}

.settings-page__scroll-inner {
  padding-bottom: 24px;
}

/* Section label */
.section-label {
  font-size: var(--font-size-caption);
  font-weight: 500;
  color: var(--color-text-muted);
  margin-bottom: 6px;
  text-transform: uppercase;
  letter-spacing: 0.5px;
}
.section-label--about {
  margin-top: 24px;
}

/* Settings card */
.settings-card {
  background: var(--color-surface-muted);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-xl);
  overflow: hidden;
  margin-bottom: 20px;
}
.settings-card--about {
  margin-bottom: 40px;
}
.dark .settings-card {
  background: var(--color-surface);
  border-color: var(--color-surface-muted);
}

/* Settings row */
.settings-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 12px 16px;
  min-height: var(--size-control-lg);
  border-bottom: 1px solid var(--color-border);
}
.settings-row:last-child {
  border-bottom: none;
}
.settings-row--interactive {
  width: 100%;
  background: none;
  border: none;
  border-bottom: 1px solid var(--color-border);
  cursor: pointer;
  padding: 12px 16px;
  min-height: var(--size-control-lg);
  display: flex;
  align-items: center;
  justify-content: space-between;
  transition: background 0.15s ease;
  -webkit-tap-highlight-color: transparent;
}
.settings-row--interactive:last-child {
  border-bottom: none;
}
.settings-row--interactive:active {
  transform: scale(0.99);
}
.settings-row--interactive:active {
  background: var(--color-surface-hover);
}
.settings-row--stacked {
  align-items: stretch;
  flex-direction: column;
  gap: 10px;
}
.github-token__header,
.github-token__actions,
.github-token__meta {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
}
.sr-icon--github {
  background: var(--color-github);
  color: var(--color-white);
  font-size: 11px;
  font-weight: 700;
}
.github-token__status {
  border-radius: 999px;
  font-size: 12px;
  font-weight: 600;
  padding: 4px 9px;
}
.github-token__status--ok {
  background: var(--color-success-soft);
  color: var(--color-success);
}
.github-token__status--empty {
  background: var(--color-warning-soft);
  color: var(--color-warning-dark);
}
.github-token__desc,
.github-token__meta {
  color: var(--color-text-muted);
  font-size: 13px;
  line-height: 1.5;
}
.github-token__input {
  width: 100%;
  min-height: var(--size-control-lg);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-xl);
  background: var(--color-surface);
  color: var(--color-text);
  /* 16px to prevent iOS Safari focus auto-zoom */
  font-size: 16px;
  padding: 12px;
  outline: none;
  transition: border-color 0.2s ease, box-shadow 0.2s ease;
}
.github-token__input:focus {
  border-color: var(--color-primary);
  box-shadow: 0 0 0 3px rgba(var(--color-primary-rgb), 0.1);
}
.github-token__btn {
  border: none;
  border-radius: var(--radius-lg);
  cursor: pointer;
  font-size: var(--font-size-label);
  font-weight: 600;
  padding: 10px 14px;
  min-height: var(--size-control-md);
  transition: transform 0.15s ease, opacity 0.15s ease;
  -webkit-tap-highlight-color: transparent;
}
.github-token__btn:active:not(:disabled) {
  transform: scale(0.98);
}
.github-token__btn:disabled {
  cursor: not-allowed;
  opacity: 0.55;
}
.github-token__btn--primary {
  background: var(--color-primary);
  color: var(--color-text-inverse);
}
.github-token__btn--danger {
  background: var(--color-danger-soft);
  color: var(--color-danger);
}
.dark .github-token__input {
  background: var(--color-surface-muted);
  border-color: var(--color-surface-muted);
  color: var(--color-text);
}
.dark .settings-row {
  border-bottom-color: var(--color-surface-muted);
}
.dark .settings-row--interactive:active {
  background: var(--color-surface);
}

/* Row left */
.sr-left {
  display: flex;
  align-items: center;
  gap: 12px;
}

/* Icon */
.sr-icon {
  width: 32px;
  height: 32px;
  border-radius: var(--radius-base);
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 15px;
  flex-shrink: 0;
}
.sr-icon--apple {
  background: var(--color-primary-soft);
}
.sr-icon--add {
  background: var(--color-surface-muted);
  font-size: 18px;
  font-weight: 700;
}
.dark .sr-icon--add {
  background: var(--color-surface-muted);
}
.sr-icon--appearance {
  background: var(--color-surface-muted);
}
.dark .sr-icon--appearance {
  background: var(--color-surface-muted);
}
.sr-icon--danger {
  background: var(--color-danger-soft);
}
.dark .sr-icon--danger {
  background: rgba(var(--rgb-red), 0.15);
}
.sr-icon--neutral {
  background: var(--color-surface-muted);
}
.dark .sr-icon--neutral {
  background: var(--color-surface-muted);
}

/* Label */
.sr-label {
  font-size: var(--font-size-section);
  color: var(--color-text);
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
}
.dark .sr-label {
  color: var(--color-text);
}
.sr-label--brand {
  color: var(--color-primary);
}
.sr-label--danger {
  color: var(--color-danger);
}

/* Account login time */
.sr-login-time {
  font-size: var(--font-size-micro);
  font-weight: 500;
  white-space: nowrap;
  color: var(--color-text-secondary);
}

/* Row right */
.sr-right {
  display: flex;
  align-items: center;
  gap: 4px;
  color: var(--color-text-muted);
  font-size: var(--font-size-body);
}
.dark .sr-right {
  color: var(--color-text-muted);
}
.sr-arrow {
  color: var(--color-text-tertiary);
}
.dark .sr-arrow {
  color: var(--color-text-tertiary);
}

.dark .section-label {
  color: var(--color-text-muted);
}

.dark .sr-icon--apple {
  background: rgba(var(--rgb-brand), 0.15);
}

/* Account action buttons */
.sr-btn {
  width: 28px;
  height: 28px;
  border-radius: var(--radius-base, 8px);
  border: none;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  font-size: 16px;
  transition: background 0.15s ease, opacity 0.15s ease;
  flex-shrink: 0;
  background: transparent;
  color: var(--color-text-muted);
}
.sr-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
.sr-btn--refresh:active:not(:disabled) {
  background: var(--color-primary-soft);
  color: var(--color-primary);
}
.sr-btn--delete {
  color: var(--color-text-muted);
}
.sr-btn--delete:active:not(:disabled) {
  background: var(--color-danger-soft);
  color: var(--color-danger);
}
.dark .sr-btn {
  color: var(--color-text-muted);
}
.dark .sr-btn--refresh:active:not(:disabled) {
  background: rgba(var(--rgb-brand), 0.15);
}
.dark .sr-btn--delete:active:not(:disabled) {
  background: rgba(var(--rgb-red), 0.15);
}

</style>
