<template>
  <div class="change-password-page">
    <!-- Top Nav Bar -->
    <div class="cp-nav">
      <button
        v-if="!forceChange"
        class="cp-nav__back"
        @click="emit('back')"
      >
        <SvgIcon
          class="cp-nav__back-icon"
          :icon="arrowLeftIcon"
        />
        返回
      </button>
      <div class="cp-nav__title">
        修改账号密码
      </div>
      <div class="cp-nav__spacer" />
    </div>

    <!-- Body -->
    <div class="cp-body">
      <!-- Green hint box -->
      <div class="cp-hint">
        <div class="cp-hint__icon">
          🔒
        </div>
        <div class="cp-hint__text">
          {{ forceChange ? '首次登录必须修改初始密码，完成后将直接进入系统。' : '修改登录账号和密码，完成后会保持当前登录态。' }}
        </div>
      </div>

      <!-- Form -->
      <form
        class="cp-form"
        @submit.prevent="handleSubmit"
      >
        <MobileInput
          v-model="form.current_password"
          type="password"
          label="当前密码"
          required
          autocomplete="current-password"
          placeholder="请输入当前密码"
          :error="errors.current_password"
        />

        <MobileInput
          v-model="form.new_username"
          label="新账号"
          placeholder="留空则不修改"
          :error="errors.new_username"
        />

        <div class="cp-field">
          <MobileInput
            v-model="form.new_password"
            type="password"
            label="新密码"
            required
            autocomplete="new-password"
            placeholder="请输入新密码（至少8位）"
            :error="errors.new_password"
          />
          <!-- Password strength bar -->
          <div
            v-if="form.new_password"
            class="strength-wrap"
          >
            <div class="strength-bar">
              <div
                v-for="i in 4"
                :key="i"
                :class="['strength-bar__segment', strengthLevel >= i ? `strength-bar__segment--${strengthClass}` : '']"
              />
            </div>
            <div
              class="strength-text"
              :style="{ color: strengthColor }"
            >
              {{ strengthLabel }}
            </div>
          </div>
        </div>

        <MobileInput
          v-model="form.confirm_password"
          type="password"
          label="确认新密码"
          required
          autocomplete="new-password"
          placeholder="请再次输入新密码"
          :error="errors.confirm_password"
        />

        <button
          type="submit"
          class="cp-submit"
          :disabled="loading"
        >
          {{ loading ? '修改中...' : '确认修改' }}
        </button>
      </form>
    </div>
  </div>
</template>

<script setup>
import SvgIcon from './SvgIcon.vue'
import arrowLeftIcon from '../assets/icons/arrow-left.svg?raw'
import { reactive, ref, computed } from 'vue'
import MobileInput from './MobileInput.vue'
import { Toast } from './MobileToast.vue'
import { apiFetch } from '../utils/api.js'

const props = defineProps({
  currentPassword: {
    type: String,
    default: ''
  },
  forceChange: {
    type: Boolean,
    default: false
  }
})

const emit = defineEmits(['back', 'success'])

const loading = ref(false)

const form = reactive({
  current_password: '',
  new_password: '',
  confirm_password: '',
  new_username: ''
})

const errors = reactive({
  current_password: '',
  new_password: '',
  confirm_password: '',
  new_username: ''
})

// Pre-fill current password if provided
if (props.currentPassword) {
  form.current_password = props.currentPassword
}

// Password strength detection
const strengthLevel = computed(() => {
  const pwd = form.new_password
  if (!pwd) return 0
  let score = 0
  if (pwd.length >= 6) score++
  if (pwd.length >= 8) score++
  if (/[A-Z]/.test(pwd) && /[a-z]/.test(pwd)) score++
  if (/[0-9]/.test(pwd)) score++
  if (/[^A-Za-z0-9]/.test(pwd)) score++
  if (pwd.length >= 12) score++
  // Map score 0-6 to level 1-4
  if (score <= 1) return 1
  if (score <= 2) return 2
  if (score <= 4) return 3
  return 4
})

const strengthClass = computed(() => {
  if (strengthLevel.value <= 1) return 'weak'
  if (strengthLevel.value <= 2) return 'medium'
  return 'strong'
})

const strengthLabel = computed(() => {
  const map = { 1: '弱', 2: '中等强度', 3: '强', 4: '非常强' }
  return map[strengthLevel.value] || ''
})

const strengthColor = computed(() => {
  const map = { 1: 'var(--color-danger)', 2: 'var(--color-warning)', 3: 'var(--color-primary)', 4: 'var(--color-primary)' }
  return map[strengthLevel.value] || 'var(--color-text-tertiary)'
})

function clearErrors() {
  errors.current_password = ''
  errors.new_password = ''
  errors.confirm_password = ''
  errors.new_username = ''
}

async function validateForm() {
  clearErrors()
  if (!form.current_password) {
    errors.current_password = '请输入当前密码'
    throw new Error('请输入当前密码')
  }
  if (!form.new_password) {
    errors.new_password = '请输入新密码'
    throw new Error('请输入新密码')
  }
  if (form.new_password.length < 8) {
    errors.new_password = '新密码长度至少8位'
    throw new Error('新密码长度至少8位')
  }
  if (!form.confirm_password) {
    errors.confirm_password = '请确认新密码'
    throw new Error('请确认新密码')
  }
  if (form.confirm_password !== form.new_password) {
    errors.confirm_password = '两次输入的密码不一致'
    throw new Error('两次输入的密码不一致')
  }
}

async function handleSubmit() {
  try {
    await validateForm()
    loading.value = true

    const { response: res, data: json } = await apiFetch('/api/auth/change-password', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      credentials: 'include',
      body: JSON.stringify({
        current_password: form.current_password,
        new_password: form.new_password,
        ...(form.new_username.trim() ? { new_username: form.new_username.trim() } : {})
      })
    })

    if (!res.ok) {
      const msg = json?.error || '修改密码失败'
      throw new Error(msg)
    }

    emit('success', json?.data || null)
  } catch (e) {
    Toast.error(e?.message || '修改密码失败')
  } finally {
    loading.value = false
  }
}
</script>

<style scoped>
.change-password-page {
  display: flex;
  flex-direction: column;
  min-height: auto;
  background: var(--color-bg-page);
}
.dark .change-password-page {
  background: var(--color-bg);
}

/* Nav bar */
.cp-nav {
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
.dark .cp-nav {
  background: var(--color-surface);
  border-bottom-color: var(--color-border);
}
.cp-nav__back {
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
.cp-nav__back-icon {
  width: 20px;
  height: 20px;
}
.cp-nav__title {
  font-size: 17px;
  font-weight: 600;
  color: var(--color-text);
}
.dark .cp-nav__title {
  color: var(--color-text);
}
.cp-nav__spacer {
  flex: 1;
}

/* Body */
.cp-body {
  flex: 1;
  display: flex;
  flex-direction: column;
  padding: 24px 20px 32px;
}

/* Hint box */
.cp-hint {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  padding: 14px 16px;
  background: var(--color-primary-soft);
  border: 1px solid var(--color-primary-border);
  border-radius: 12px;
  margin-bottom: 24px;
}
.cp-hint__icon {
  font-size: 16px;
  line-height: 1.4;
  flex-shrink: 0;
}
.cp-hint__text {
  font-size: var(--font-size-label);
  line-height: 1.5;
  color: var(--color-text-tag);
}
.dark .cp-hint {
  background: rgba(var(--rgb-brand), 0.1);
  border-color: rgba(var(--rgb-brand), 0.3);
}
.dark .cp-hint__text {
  color: var(--color-text);
}

/* Form */
.cp-form {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}

.cp-field {
  display: flex;
  flex-direction: column;
}

/* Strength bar */
.strength-wrap {
  margin-top: 8px;
  display: flex;
  align-items: center;
  gap: 8px;
}
.strength-bar {
  display: flex;
  gap: 4px;
  flex: 1;
}
.strength-bar__segment {
  height: 4px;
  flex: 1;
  border-radius: 2px;
  background: var(--color-border-light);
  transition: background 0.2s ease;
}
.dark .strength-bar__segment {
  background: var(--color-border);
}
.strength-bar__segment--weak {
  background: var(--color-danger);
}
.strength-bar__segment--medium {
  background: var(--color-warning);
}
.strength-bar__segment--strong {
  background: var(--color-primary);
}
.strength-text {
  font-size: var(--font-size-caption);
  font-weight: 500;
  white-space: nowrap;
}

/* Submit button */
.cp-submit {
  width: 100%;
  min-height: var(--size-control-lg);
  padding: 14px;
  border-radius: var(--radius-xl);
  border: none;
  background: var(--color-primary);
  color: var(--color-text-inverse);
  font-size: var(--font-size-section);
  font-weight: 600;
  cursor: pointer;
  margin-top: 24px;
  transition: background 0.15s ease, transform 0.15s ease;
  -webkit-tap-highlight-color: transparent;
}
.cp-submit:active:not(:disabled) {
  background: var(--color-primary-active);
  transform: scale(0.98);
}
.cp-submit:disabled {
  background: var(--color-text-disabled);
  cursor: not-allowed;
}
</style>
