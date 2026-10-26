<template>
  <div class="appearance-page">
    <!-- Secondary page navigation bar -->
    <div class="ap-nav">
      <button
        class="ap-nav__back"
        @click="emit('close')"
      >
        <SvgIcon
          class="ap-nav__back-icon"
          :icon="arrowLeftIcon"
        />
        返回
      </button>
      <div class="ap-nav__title">
        {{ isRelogin ? '重新登录 Apple ID' : '添加 Apple ID' }}
      </div>
      <div class="ap-nav__spacer" />
    </div>

    <div class="ap-body">
      <div class="ap-section">
        <div class="ap-section__title">
          登录 Apple ID
        </div>
        <div class="ap-card ap-card--form">
          <div class="ap-form">
            <MobileInput
              v-model="newAccount.email"
              type="email"
              label="邮箱"
              placeholder="your@email.com"
              :disabled="logging"
              clearable
              autocomplete="email"
            />

            <MobileInput
              v-model="newAccount.password"
              type="password"
              label="密码"
              :placeholder="hasSavedPassword ? '已保存，可留空' : '••••••••'"
              :disabled="logging"
              autocomplete="current-password"
            />

            <MobileInput
              v-model="newAccount.memo"
              type="text"
              label="备注（可选）"
              placeholder="例：工作号、美区小号..."
              :disabled="logging"
              clearable
            />

            <div :class="{ 'ring-2 ring-brand ring-offset-2 dark:ring-offset-surface-dark-page rounded-xl': mfaRequired }">
              <MobileInput
                v-model="newAccount.code"
                type="text"
                label="验证码"
                placeholder="两步验证码（如需要）"
                :disabled="logging"
                clearable
                autocomplete="one-time-code"
                inputmode="numeric"
                :hint="mfaRequired ? '请输入受信任设备上收到的 6 位验证码' : ''"
              />
            </div>

            <label
              class="mobile-checkbox"
              :class="{ 'mobile-checkbox--disabled': logging }"
            >
              <input
                type="checkbox"
                class="mobile-checkbox__input"
                :checked="savePassword"
                :disabled="logging"
                @change="savePassword = $event.target.checked"
              >
              <span class="mobile-checkbox__box">
                <SvgIcon
                  class="mobile-checkbox__check"
                  :icon="checkboxCheckIcon"
                />
              </span>
              <span class="mobile-checkbox__label text-[14px] text-txt dark:text-txt-dark">保存密码，重新登录时只需输入验证码</span>
            </label>

            <MobileButton
              :disabled="logging || !isFormValid"
              :loading="logging"
              type="primary"
              block
              class="!rounded-[12px]"
              @click="loginAccount"
            >
              {{ logging ? '登录中...' : (isRelogin ? '重新登录' : '添加账号') }}
            </MobileButton>

            <p class="ap-form__hint">
              App Store 区域将根据账号信息自动匹配
            </p>
          </div>
        </div>
      </div>

      <div style="height: 40px;" />
    </div>
  </div>
</template>

<script setup>
import SvgIcon from './SvgIcon.vue'
import arrowLeftIcon from '../assets/icons/arrow-left.svg?raw'
import checkboxCheckIcon from '../assets/icons/checkbox-check.svg?raw'
import { ref, computed, onMounted } from 'vue'
import { API_BASE } from '../config.js'

import MobileButton from './MobileButton.vue'
import MobileInput from './MobileInput.vue'
import { Toast } from './MobileToast.vue'
import { useAccounts, dedupeAccounts } from '../composables/useAccounts.js'
import { apiFetch } from '../utils/api.js'

const props = defineProps({
  // 重新登录时预填的邮箱
  reloginEmail: {
    type: String,
    default: ''
  }
})

const emit = defineEmits(['accounts-updated', 'close'])

const { accounts, loadAccounts, loadSavedCredentials: useLoadSavedCredentials, persistAccounts } = useAccounts()
const savedCredentials = ref([])
const isRelogin = computed(() => !!props.reloginEmail)
const newAccount = ref({
  email: props.reloginEmail,
  password: '',
  code: '',
  memo: '',
})
const logging = ref(false)
const savePassword = ref(true)
const mfaRequired = ref(false)

const hasSavedPassword = computed(() => {
  const email = newAccount.value.email.trim().toLowerCase()
  return !!email && savedCredentials.value.some((c) => c.email?.toLowerCase() === email)
})

const isFormValid = computed(() => {
  return newAccount.value.email && (newAccount.value.password || hasSavedPassword.value)
})

const loadSavedCredentials = async () => {
  const creds = await useLoadSavedCredentials()
  if (creds) {
    savedCredentials.value = creds
  }
}


const saveAccounts = () => {
  persistAccounts()
  emit('accounts-updated', accounts.value)
}

const loginAccount = async () => {
  if (!isFormValid.value) {
    Toast.warning('请填写完整的账号信息')
    return
  }

  const existingAccount = accounts.value.find(
    (acc) => acc.email === newAccount.value.email,
  )
  if (existingAccount && !isRelogin.value) {
    Toast.warning('该账号已登录，如需更新登录态请在设置中点击重新登录')
    return
  }

  logging.value = true

  try {
    // 签名登录遇到 Apple 临时错误会退避重试，可能超过默认 30s；登录不可重放，不自动重试
    const { response, data } = await apiFetch(`${API_BASE}/login`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({
        email: newAccount.value.email,
        password: newAccount.value.password,
        mfa: newAccount.value.code || undefined,
        saveCredentials: savePassword.value,
        memo: newAccount.value.memo || undefined,
      }),
    }, { timeout: 120000, retries: 0 })

    if (!response.ok && !data.ok) {
      Toast.error(`登录失败：${data.error || '服务器错误'}`)
      logging.value = false
      return
    }

    if (data.ok && data.data?.status === 'need_mfa') {
      mfaRequired.value = true
      Toast.show({
        type: 'warning',
        message: '此账号需要二次验证，请查看你的受信任设备上的验证码，填入后再次点击登录',
        duration: 8000,
      })
      logging.value = false
      return
    }

    if (data.ok && data.data?.status === 'mfa_failed') {
      Toast.error('验证码无效或已过期，请重新输入')
      newAccount.value.code = ''
      logging.value = false
      return
    }

    if (!data.ok) {
      const errMsg = data.error || '未知错误'
      Toast.error(`登录失败：${errMsg}`)
      if (errMsg.includes('密码') || errMsg.includes('BadLogin')) {
        mfaRequired.value = true
      }
      logging.value = false
      return
    }

    mfaRequired.value = false
    accounts.value = dedupeAccounts([
      ...accounts.value.filter((acc) => acc.email !== data.data.email),
      {
        token: data.data.token,
        email: data.data.email,
        dsid: data.data.dsid,
        region: data.data.region || 'US',
        hasSavedCredentials: !!savePassword.value,
        loggedInAt: Math.floor(Date.now() / 1000),
      }
    ])

    await loadSavedCredentials()
    saveAccounts()

    newAccount.value = { email: '', password: '', code: '', memo: '' }

    Toast.success(`登录成功：${data.data.email}`)
    if (isRelogin.value) emit('close')
  } catch (error) {
    Toast.error(`网络错误：${error.message}`)
  } finally {
    logging.value = false
  }
}

onMounted(async () => {
  await loadSavedCredentials()
  await loadAccounts()
  emit('accounts-updated', accounts.value)
})

defineExpose({
  accounts,
})
</script>

<style scoped>
.appearance-page {
  display: flex;
  flex-direction: column;
  min-height: 100svh;
  min-height: 100dvh;
  background: var(--color-bg-page);
}

.dark .appearance-page {
  background: var(--color-bg);
}

.ap-nav {
  position: sticky;
  top: 0;
  z-index: 10;
  display: flex;
  align-items: center;
  gap: 12px;
  min-height: calc(56px + env(safe-area-inset-top, 0px));
  margin: 0 calc(var(--space-5) * -1) 20px;
  padding: env(safe-area-inset-top, 0px) var(--space-5) 0;
  background: var(--color-bg-white);
  border-bottom: 1px solid var(--color-border-light);
  flex-shrink: 0;
}

.dark .ap-nav {
  background: var(--color-surface);
  border-bottom-color: var(--color-border);
}

.ap-nav__back {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  border: none;
  background: transparent;
  padding: 0;
  font-size: 15px;
  font-weight: 500;
  color: var(--color-primary);
  cursor: pointer;
}

.ap-nav__back-icon {
  width: 20px;
  height: 20px;
}

.ap-nav__title {
  font-size: 17px;
  font-weight: 600;
  color: var(--color-text);
}

.dark .ap-nav__title {
  color: var(--color-text);
}

.ap-nav__spacer {
  flex: 1;
}

.ap-body {
  flex: 1;
  overflow-y: auto;
}

.ap-section {
  padding: var(--space-5) var(--space-5) 0;
}

.ap-section__title {
  font-size: var(--font-size-caption);
  font-weight: 500;
  color: var(--color-text-muted);
  text-transform: uppercase;
  letter-spacing: 0.5px;
  margin-bottom: 6px;
}

.dark .ap-section__title {
  color: var(--color-text-muted);
}

.ap-card {
  background: var(--color-surface-muted);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-xl);
  overflow: hidden;
}

.dark .ap-card {
  background: var(--color-surface);
  border-color: var(--color-surface-muted);
}

.ap-card--form {
  padding: 16px;
}

.ap-form {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.ap-form__hint {
  padding-top: 8px;
  text-align: center;
  font-size: var(--font-size-caption);
  color: var(--color-text-tertiary);
}

.dark .ap-form__hint {
  color: var(--color-text-tertiary);
}

/* Inline checkbox styles for save-password */
.mobile-checkbox {
  display: inline-flex;
  align-items: center;
  gap: 10px;
  cursor: pointer;
  user-select: none;
  -webkit-tap-highlight-color: transparent;
  min-height: 22px;
  padding: 2px 0;
  box-sizing: border-box;
}

.mobile-checkbox__input {
  position: absolute;
  width: 1px;
  height: 1px;
  margin: -1px;
  padding: 0;
  overflow: hidden;
  clip: rect(0, 0, 0, 0);
  border: 0;
}

.mobile-checkbox__box {
 position: relative;
 display: inline-flex;
 align-items: center;
 justify-content: center;
 width: 22px;
 height: 22px;
 border: 2px solid var(--color-border-strong);
 border-radius: var(--radius-base);
 background: var(--color-surface);
 transition: all 0.2s ease;
 flex-shrink: 0;
}

.mobile-checkbox__check {
  width: 12px;
  height: 10px;
  color: var(--color-text-inverse);
  opacity: 0;
  transform: scale(0.5);
  transition: all 0.2s cubic-bezier(0.4, 0, 0.2, 1);
}

:deep(.mobile-checkbox--checked) .mobile-checkbox__box,
.mobile-checkbox:has(.mobile-checkbox__input:checked) .mobile-checkbox__box {
 background: var(--color-primary);
 border-color: var(--color-primary);
}

:deep(.mobile-checkbox--checked) .mobile-checkbox__check,
.mobile-checkbox:has(.mobile-checkbox__input:checked) .mobile-checkbox__check {
 opacity: 1;
 transform: scale(1);
}

.mobile-checkbox__label {
 font-size: 14px;
 font-family: -apple-system, 'SF Pro Display', 'Helvetica Neue', sans-serif;
 color: var(--color-text);
 line-height: 1.4;
}

.mobile-checkbox--disabled {
 opacity: 0.5;
 cursor: not-allowed;
 pointer-events: none;
}

/* Dark mode for inline checkbox */
.dark .mobile-checkbox__box {
 background: rgba(var(--rgb-white), 0.08);
 border-color: rgba(var(--rgb-white), 0.2);
}

.dark .mobile-checkbox:has(.mobile-checkbox__input:checked) .mobile-checkbox__box {
 background: var(--color-primary);
 border-color: var(--color-primary);
}

.dark .mobile-checkbox__label {
 color: var(--color-text);
}
</style>
