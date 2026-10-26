<template>
  <div class="login-shell flex items-center justify-center bg-surface-page dark:bg-surface-dark-page px-5 py-10">
    <div
      v-if="viewMode === 'login'"
      class="w-full max-w-[420px]"
    >
      <!-- Brand Area -->
      <div class="mb-8 flex flex-col items-center text-center">
        <div class="brand-icon">
          i
        </div>
        <h1 class="mb-1 text-[20px] font-bold text-txt dark:text-txt-dark">
          ipaTool
        </h1>
        <p class="text-[14px] text-txt-secondary dark:text-txt-dark-secondary">
          IPA 下载 · 签名 · 管理
        </p>
      </div>

      <!-- Login Form -->
      <form
        class="login-form space-y-3"
        @submit.prevent="handleLogin"
      >
        <MobileInput
          v-model="loginForm.username"
          label="用户名"
          required
          autocomplete="username"
          placeholder="请输入用户名"
          :error="loginErrors.username"
          @keyup.enter="handleLogin"
        />

        <MobileInput
          v-model="loginForm.password"
          type="password"
          label="密码"
          required
          autocomplete="current-password"
          placeholder="请输入密码"
          :error="loginErrors.password"
          @keyup.enter="handleLogin"
        />

        <button
          type="submit"
          class="hidden"
          aria-hidden="true"
        />

        <button
          type="button"
          class="login-btn"
          :disabled="loginLoading"
          @click="handleLogin"
        >
          {{ loginLoading ? '登录中...' : '登录' }}
        </button>
      </form>

      <!-- Footer -->
      <div class="mt-6 text-center">
        <p class="mb-2 text-[12px] text-txt-tertiary dark:text-txt-dark-tertiary">
          首次登录需要修改初始密码
        </p>
        <p class="text-[12px] text-txt-tertiary dark:text-txt-dark-tertiary">
          v{{ appVersion }}
        </p>
      </div>
    </div>

    <!-- Change Password Page (viewMode === 'changepassword') -->
    <ChangePassword
      v-else-if="viewMode === 'changepassword'"
      :current-password="loginForm.password"
      :force-change="forcePasswordChange"
      @back="handleChangePasswordBack"
      @success="handleChangePasswordSuccess"
    />
  </div>
</template>

<script setup>
/* global __APP_VERSION__ */
import { reactive, ref } from 'vue'
import { useAppStore } from '@/stores/app'
import MobileInput from './MobileInput.vue'
import ChangePassword from './ChangePassword.vue'
import { Toast } from './MobileToast.vue'

const props = defineProps({
  forcePasswordChange: {
    type: Boolean,
    default: false
  }
})

const emit = defineEmits(['login-success'])
const appStore = useAppStore()
const appVersion = __APP_VERSION__

// View mode: 'login' | 'changepassword'
const viewMode = ref(props.forcePasswordChange ? 'changepassword' : 'login')

const loginLoading = ref(false)

const loginForm = reactive({
  username: '',
  password: ''
})

const loginErrors = reactive({
  username: '',
  password: ''
})

function clearLoginErrors() {
  loginErrors.username = ''
  loginErrors.password = ''
}

async function validateLoginForm() {
  clearLoginErrors()
  if (!loginForm.username) {
    loginErrors.username = '请输入用户名'
    throw new Error('请输入用户名')
  }
  if (!loginForm.password) {
    loginErrors.password = '请输入密码'
    throw new Error('请输入密码')
  }
}

const handleLogin = async () => {
  try {
    await validateLoginForm()

    loginLoading.value = true
    const user = await appStore.loginAdmin(loginForm.username, loginForm.password)

    if (user?.is_default) {
      viewMode.value = 'changepassword'
    } else {
      Toast.success('登录成功')
      emit('login-success')
    }
  } catch (e) {
    Toast.error(e?.message || '登录失败')
  } finally {
    loginLoading.value = false
  }
}

const handleChangePasswordBack = () => {
  if (!props.forcePasswordChange) {
    viewMode.value = 'login'
  }
}

const handleChangePasswordSuccess = async (user) => {
  if (user) {
    appStore.setAuthUser(user)
  }
  Toast.success('账号密码修改成功')
  emit('login-success')
}
</script>

<style scoped>
.login-shell {
  min-height: 100svh;
  min-height: 100dvh;
  /* Avoid soft-keyboard overlap on iOS Safari (set by useKeyboardAware) */
  padding-bottom: max(40px, var(--kb-inset-bottom, 0px));
}

/* Brand icon — match home card-icon radius */
.brand-icon {
  width: 64px;
  height: 64px;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: var(--radius-xl);
  background: var(--color-primary);
  color: var(--color-text-inverse);
  font-size: 28px;
  font-weight: 700;
  margin-bottom: var(--space-3);
}

/* Login button (matches MobileButton primary) */
.login-btn {
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
  margin-top: 8px;
  transition: background 0.15s ease, transform 0.15s ease;
  -webkit-tap-highlight-color: transparent;
}
.login-btn:active:not(:disabled) {
  background: var(--color-primary-hover);
  transform: scale(0.98);
}
.login-btn:disabled {
  background: var(--color-text-disabled);
  cursor: not-allowed;
}
</style>
