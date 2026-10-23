<template>
  <p class="section-label text-txt-secondary dark:text-txt-dark-secondary">
    WebDAV 自动上传
  </p>
  <div class="settings-card settings-card--webdav">
    <div class="settings-row settings-row--stacked">
      <!-- Header + Enable Toggle -->
      <div class="webdav__header">
        <div class="sr-left">
          <div class="sr-icon sr-icon--webdav">
            WD
          </div>
          <div class="sr-label">
            WebDAV
          </div>
        </div>
        <button
          class="webdav__toggle"
          :class="{ 'webdav__toggle--on': form.enabled }"
          type="button"
          role="switch"
          :aria-checked="form.enabled"
          @click="form.enabled = !form.enabled"
        >
          <span class="webdav__toggle-knob" />
        </button>
      </div>
      <div class="webdav__desc">
        开启后，每次下载完成自动把 IPA 上传到指定的 WebDAV 目录。密码只保存在后端，前端不回显明文。
      </div>

      <!-- Config Fields -->
      <div class="webdav__fields">
        <label class="webdav__field">
          <span class="webdav__field-label">服务器地址</span>
          <input
            v-model="form.url"
            class="webdav__input"
            type="text"
            spellcheck="false"
            placeholder="https://dav.example.com/"
            autocomplete="url"
          >
        </label>
        <label class="webdav__field">
          <span class="webdav__field-label">用户名</span>
          <input
            v-model="form.username"
            class="webdav__input"
            type="text"
            spellcheck="false"
            placeholder="WebDAV 用户名"
            autocomplete="username"
          >
        </label>
        <label class="webdav__field">
          <span class="webdav__field-label">
            密码
            <span
              v-if="passwordConfigured"
              class="webdav__field-hint"
            >（已配置，留空保持不变）</span>
          </span>
          <input
            v-model="form.password"
            class="webdav__input"
            type="password"
            spellcheck="false"
            :placeholder="passwordConfigured ? maskedPassword : '输入新密码'"
            autocomplete="new-password"
          >
        </label>
        <label class="webdav__field">
          <span class="webdav__field-label">远程目录</span>
          <div class="webdav__path-row">
            <input
              v-model="form.remote_path"
              class="webdav__input webdav__input--path"
              type="text"
              spellcheck="false"
              placeholder="/ipa-downloads"
            >
            <button
              class="webdav__browse-btn"
              type="button"
              :disabled="browsing || !form.url.trim()"
              @click="openBrowser('/')"
            >
              {{ browsing ? '加载中…' : '浏览' }}
            </button>
          </div>

          <!-- Directory Browser Panel -->
          <div
            v-if="showBrowser"
            class="webdav__browser"
          >
            <div class="webdav__browser-header">
              <button
                class="webdav__browser-nav"
                :disabled="browserHistory.length === 0"
                @click="goUp"
              >
                ← 上级
              </button>
              <span class="webdav__browser-path">{{ browserPath }}</span>
            </div>
            <div class="webdav__browser-body">
              <div
                v-if="browseError"
                class="webdav__browse-error"
              >
                {{ browseError }}
              </div>
              <div
                v-for="entry in entries"
                :key="entry.path"
                class="webdav__browser-entry"
                :class="{ 'webdav__browser-entry--dir': entry.is_dir }"
                @click="entry.is_dir ? navigate(entry.path) : null"
              >
                <span class="webdav__browser-icon">{{ entry.is_dir ? '📁' : '📄' }}</span>
                <span class="webdav__browser-name">{{ entry.name }}</span>
              </div>
              <div
                v-if="!browseError && entries.length === 0 && !browsing"
                class="webdav__browse-empty"
              >
                目录为空
              </div>
            </div>
            <div class="webdav__browser-footer">
              <button
                class="webdav__btn webdav__btn--primary webdav__browser-select"
                @click="selectCurrent"
              >
                选择此目录
              </button>
              <button
                class="webdav__btn webdav__btn--secondary"
                @click="showBrowser = false"
              >
                取消
              </button>
            </div>
          </div>
        </label>
      </div>

      <!-- Test Result -->
      <div
        v-if="testResult"
        class="webdav__test-result"
        :class="testResult.ok ? 'webdav__test-result--ok' : 'webdav__test-result--fail'"
      >
        {{ testResult.ok ? '✓ ' : '✕ ' }}{{ testResult.message }}
      </div>

      <!-- Actions -->
      <div class="webdav__actions">
        <button
          class="webdav__btn webdav__btn--secondary"
          :disabled="testing || !form.url.trim()"
          @click="handleTest"
        >
          {{ testing ? '测试中…' : '测试连接' }}
        </button>
        <button
          class="webdav__btn webdav__btn--primary"
          :disabled="saving"
          @click="handleSave"
        >
          {{ saving ? '保存中…' : '保存配置' }}
        </button>
      </div>
    </div>
  </div>
</template>

<script setup>
import { onMounted, reactive, ref } from 'vue'
import { API_BASE } from '../config.js'
import { apiFetch } from '../utils/api.js'

const form = reactive({
  enabled: false,
  url: '',
  username: '',
  password: '',
  remote_path: '/',
})
const passwordConfigured = ref(false)
const maskedPassword = ref('')
const saving = ref(false)
const testing = ref(false)
const testResult = ref(null)

// Directory browser state
const showBrowser = ref(false)
const browserPath = ref('/')
const entries = ref([])
const browsing = ref(false)
const browseError = ref(null)
const browserHistory = ref([])

onMounted(fetchConfig)

async function fetchConfig() {
  try {
    const { data } = await apiFetch(`${API_BASE}/webdav/config`)
    if (data?.ok && data.data) {
      form.enabled = data.data.enabled || false
      form.url = data.data.url || ''
      form.username = data.data.username || ''
      form.remote_path = data.data.remote_path || '/'
      passwordConfigured.value = data.data.password_configured || false
      maskedPassword.value = data.data.password_masked || ''
      form.password = ''
    }
  } catch {
    // 静默失败
  }
}

async function handleSave() {
  saving.value = true
  testResult.value = null
  try {
    const { data } = await apiFetch(`${API_BASE}/webdav/config`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ ...form }),
    })
    if (data?.ok) {
      form.password = ''
      await fetchConfig()
    }
  } finally {
    saving.value = false
  }
}

async function handleTest() {
  testing.value = true
  testResult.value = null
  try {
    const { data } = await apiFetch(`${API_BASE}/webdav/test`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ ...form }),
    })
    if (data?.ok && data.data) {
      testResult.value = data.data
    }
  } finally {
    testing.value = false
  }
}

// Directory browser
function buildBrowsePayload() {
  return {
    url: form.url,
    username: form.username,
    password: form.password,
    remote_path: browserPath.value,
  }
}

async function openBrowser(path) {
  showBrowser.value = true
  browserHistory.value = []
  await loadDirectory(path)
}

async function loadDirectory(path) {
  browsing.value = true
  browseError.value = null
  entries.value = []
  browserPath.value = path
  try {
    const { data } = await apiFetch(`${API_BASE}/webdav/browse`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(buildBrowsePayload()),
    })
    if (data?.ok && data.data) {
      if (data.data.entries) {
        entries.value = data.data.entries
      } else if (data.data.ok === false) {
        browseError.value = data.data.message
      }
    }
  } catch (e) {
    browseError.value = `请求失败: ${e.message || e}`
  } finally {
    browsing.value = false
  }
}

function navigate(path) {
  browserHistory.value.push(browserPath.value)
  const newPath = browserPath.value === '/'
    ? `/${path}`
    : `${browserPath.value.replace(/\/$/, '')}/${path}`
  loadDirectory(newPath)
}

function goUp() {
  if (browserHistory.value.length === 0) return
  const prev = browserHistory.value.pop()
  loadDirectory(prev)
}

function selectCurrent() {
  form.remote_path = browserPath.value
  showBrowser.value = false
}
</script>

<style scoped>
.settings-card--webdav {
  margin-bottom: 0;
}

.webdav__header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 6px;
}
.webdav__desc {
  font-size: 12px;
  color: var(--color-text-secondary, #6e6e80);
  line-height: 1.5;
  margin-bottom: 14px;
}
.dark .webdav__desc {
  color: var(--color-text-muted, #a1a1aa);
}

.sr-icon--webdav {
  background: rgba(16, 163, 127, 0.12);
  color: var(--color-primary, #10a37f);
}

/* Toggle switch */
.webdav__toggle {
  position: relative;
  width: 44px;
  height: 26px;
  border-radius: 13px;
  background: var(--color-border, #d4d4d8);
  border: none;
  cursor: pointer;
  transition: background 0.2s ease;
  padding: 0;
  flex-shrink: 0;
}
.dark .webdav__toggle {
  background: var(--color-border, #3f3f46);
}
.webdav__toggle--on {
  background: var(--color-primary, #10a37f);
}
.webdav__toggle-knob {
  position: absolute;
  top: 3px;
  left: 3px;
  width: 20px;
  height: 20px;
  border-radius: 50%;
  background: #fff;
  transition: transform 0.2s ease;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.2);
}
.webdav__toggle--on .webdav__toggle-knob {
  transform: translateX(18px);
}

/* Fields */
.webdav__fields {
  display: flex;
  flex-direction: column;
  gap: 12px;
  margin-bottom: 14px;
}
.webdav__field {
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.webdav__field-label {
  font-size: 12px;
  font-weight: 600;
  color: var(--color-text-secondary, #6e6e80);
}
.dark .webdav__field-label {
  color: var(--color-text-muted, #a1a1aa);
}
.webdav__field-hint {
  font-weight: 400;
  opacity: 0.7;
}
.webdav__input {
  width: 100%;
  padding: 10px 12px;
  border-radius: 8px;
  border: 1px solid var(--color-border, #d4d4d8);
  background: var(--color-surface-muted, #f7f7f8);
  font-size: 16px;
  color: var(--color-text, #0d0d0d);
  outline: none;
  transition: border-color 0.15s ease;
  box-sizing: border-box;
}
.dark .webdav__input {
  background: var(--color-surface-muted, #27272a);
  border-color: var(--color-border, #3f3f46);
  color: var(--color-text, #f5f5f5);
}
.webdav__input:focus {
  border-color: var(--color-primary, #10a37f);
}
.webdav__input::placeholder {
  color: var(--color-text-muted, #a1a1aa);
}

/* Path row */
.webdav__path-row {
  display: flex;
  gap: 8px;
}
.webdav__input--path {
  flex: 1;
}
.webdav__browse-btn {
  flex-shrink: 0;
  padding: 8px 14px;
  min-height: 40px;
  border-radius: 8px;
  border: 1px solid var(--color-border, #d4d4d8);
  background: var(--color-surface-muted, #f7f7f8);
  color: var(--color-text, #0d0d0d);
  font-size: 14px;
  font-weight: 500;
  cursor: pointer;
  transition: opacity 0.15s ease;
  white-space: nowrap;
  touch-action: manipulation;
  -webkit-tap-highlight-color: transparent;
}
.dark .webdav__browse-btn {
  background: var(--color-surface-muted, #27272a);
  border-color: var(--color-border, #3f3f46);
  color: var(--color-text, #f5f5f5);
}
.webdav__browse-btn:disabled {
  opacity: 0.5;
  cursor: default;
}

/* Browser */
.webdav__browser {
  margin-top: 8px;
  border: 1px solid var(--color-border, #d4d4d8);
  border-radius: 10px;
  overflow: hidden;
  background: var(--color-surface, #fff);
}
.dark .webdav__browser {
  background: var(--color-surface, #1a1a2e);
  border-color: var(--color-border, #3f3f46);
}
.webdav__browser-header {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 12px;
  border-bottom: 1px solid var(--color-border, #d4d4d8);
  background: var(--color-surface-muted, #f7f7f8);
}
.dark .webdav__browser-header {
  background: var(--color-surface-muted, #27272a);
  border-color: var(--color-border, #3f3f46);
}
.webdav__browser-nav {
  flex-shrink: 0;
  padding: 4px 10px;
  border-radius: 6px;
  border: 1px solid var(--color-border, #d4d4d8);
  background: var(--color-surface, #fff);
  color: var(--color-text, #0d0d0d);
  font-size: 12px;
  cursor: pointer;
}
.dark .webdav__browser-nav {
  background: var(--color-surface, #1a1a2e);
  border-color: var(--color-border, #3f3f46);
  color: var(--color-text, #f5f5f5);
}
.webdav__browser-nav:disabled {
  opacity: 0.4;
  cursor: default;
}
.webdav__browser-path {
  font-size: 12px;
  font-family: monospace;
  color: var(--color-text-secondary, #6e6e80);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.dark .webdav__browser-path {
  color: var(--color-text-muted, #a1a1aa);
}
.webdav__browser-body {
  max-height: 200px;
  overflow-y: auto;
  padding: 4px 0;
}
.webdav__browser-entry {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 12px;
  cursor: default;
  font-size: 13px;
  color: var(--color-text, #0d0d0d);
  transition: background 0.1s ease;
}
.dark .webdav__browser-entry {
  color: var(--color-text, #f5f5f5);
}
.webdav__browser-entry--dir {
  cursor: pointer;
}
.webdav__browser-entry--dir:hover {
  background: var(--color-surface-muted, #f7f7f8);
}
.dark .webdav__browser-entry--dir:hover {
  background: var(--color-surface-muted, #27272a);
}
.webdav__browser-icon {
  font-size: 14px;
  flex-shrink: 0;
}
.webdav__browser-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.webdav__browse-empty,
.webdav__browse-error {
  padding: 20px 12px;
  text-align: center;
  font-size: 12px;
  color: var(--color-text-muted, #a1a1aa);
}
.webdav__browse-error {
  color: var(--color-danger, #ef4444);
}
.webdav__browser-footer {
  display: flex;
  gap: 8px;
  padding: 10px 12px;
  border-top: 1px solid var(--color-border, #d4d4d8);
}
.dark .webdav__browser-footer {
  border-color: var(--color-border, #3f3f46);
}
.webdav__browser-select {
  flex: 1;
}

/* Test result */
.webdav__test-result {
  font-size: 12px;
  padding: 8px 12px;
  border-radius: 8px;
  margin-bottom: 12px;
}
.webdav__test-result--ok {
  background: rgba(16, 163, 127, 0.1);
  color: var(--color-primary, #10a37f);
}
.webdav__test-result--fail {
  background: rgba(239, 68, 68, 0.1);
  color: var(--color-danger, #ef4444);
}

/* Actions */
.webdav__actions {
  display: flex;
  gap: 8px;
}
.webdav__btn {
  flex: 1;
  padding: 9px 16px;
  border-radius: 9px;
  border: none;
  font-size: 13px;
  font-weight: 600;
  cursor: pointer;
  transition: opacity 0.15s ease;
}
.webdav__btn:disabled {
  opacity: 0.5;
  cursor: default;
}
.webdav__btn--primary {
  background: var(--color-primary, #10a37f);
  color: #fff;
}
.webdav__btn--secondary {
  background: var(--color-surface-muted, #f7f7f8);
  color: var(--color-text, #0d0d0d);
  border: 1px solid var(--color-border, #d4d4d8);
}
.dark .webdav__btn--secondary {
  background: var(--color-surface-muted, #27272a);
  color: var(--color-text, #f5f5f5);
  border-color: var(--color-border, #3f3f46);
}
</style>