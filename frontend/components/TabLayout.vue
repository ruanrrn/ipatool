<template>
  <div
    class="tab-layout"
    :class="{ 'mobile-layout': isMobile }"
  >
    <div
      v-if="!isMobile"
      class="desktop-tabs"
    >
      <button
        v-for="tab in tabs"
        :key="tab.id"
        :class="['desktop-tab', appStore.activeTab === tab.id ? 'is-active' : '']"
        @click="selectTab(tab.id)"
      >
        <MobileBadge
          v-if="tab.badge"
          :value="tab.badge"
        >
          <SvgIcon
            class="tab-layout__icon h-6 w-6"
            :icon="tab.icon"
          />
        </MobileBadge>
        <SvgIcon
          v-else
          class="tab-layout__icon h-6 w-6"
          :icon="tab.icon"
        />
        <span>{{ tab.label }}</span>
      </button>
    </div>

    <div
      class="tab-content"
      :class="{ 'with-mobile-tabs': isMobile }"
    >
      <div class="subpage-host">
        <!-- Sub page: Appearance -->
        <Appearance
          v-if="subPage === 'appearance'"
          key="appearance"
          @back="subPage = null"
        />

        <!-- Sub page: Account Manager -->
        <AccountManager
          v-else-if="subPage === 'account'"
          key="account"
          :relogin-email="reloginEmail"
          @close="subPage = null"
          @accounts-updated="(v) => emit('accounts-updated', v)"
        />

        <!-- Sub page: Change Password -->
        <ChangePassword
          v-else-if="subPage === 'changepassword'"
          key="changepassword"
          @back="subPage = null"
          @success="emit('logout')"
        />

        <!-- Normal tab content -->
        <div
          v-else
          key="tabs"
          class="tab-host"
        >
          <KeepAlive>
            <component
              :is="currentTabComponent"
              :key="appStore.activeTab"
              v-bind="currentTabProps"
              @app-selected="handleAppSelected"
              @download-started="handleDownloadStarted"
              @accounts-updated="handleAccountsUpdated"
              @remove-item="emit('remove-item', $event)"
              @clear-all="emit('clear-queue')"
              @logout="emit('logout')"
              @navigate-to-appearance="subPage = 'appearance'"
              @navigate-to-account="openAccountPage"
              @navigate-to-changepassword="subPage = 'changepassword'"
            />
          </KeepAlive>
        </div>
      </div>
    </div>

    <!-- Orbit v3 Mobile Tab Bar -->
    <div
      v-if="isMobile"
      class="mobile-tabs-wrap"
    >
      <div class="mobile-tabs">
        <button
          v-for="tab in tabs"
          :key="tab.id"
          :class="['mobile-tab', appStore.activeTab === tab.id ? 'is-active' : '']"
          @click="selectTab(tab.id)"
        >
          <div class="mobile-tab__icon-wrap">
            <SvgIcon
              class="tab-layout__icon h-6 w-6"
              :icon="tab.icon"
            />
            <!-- Badge for queue tab -->
            <span
              v-if="tab.badge"
              class="mobile-tab__badge"
            >{{ tab.badge }}</span>
          </div>
          <span class="mobile-tab__label">{{ tab.label }}</span>
        </button>
      </div>
    </div>
  </div>
</template>

<script setup>
import { computed, defineAsyncComponent, onMounted, onUnmounted, ref, watch } from 'vue'
import MobileBadge from './MobileBadge.vue'
import SvgIcon from './SvgIcon.vue'
import { useAppStore } from '../stores/app'
import DownloadManager from './DownloadManager.vue'
import homeIcon from '../assets/icons/home.svg?raw'

const Appearance = defineAsyncComponent(() => import('./Appearance.vue'))
const AccountManager = defineAsyncComponent(() => import('./AccountManager.vue'))
import ChangePassword from './ChangePassword.vue'
const IpaManager = defineAsyncComponent(() => import('./IpaManager.vue'))
const ArchiveApp = defineAsyncComponent(() => import('./ArchiveApp.vue'))
const Settings = defineAsyncComponent(() => import('./Settings.vue'))

import clockIcon from '../assets/icons/clock.svg?raw'
import bookmarkIcon from '../assets/icons/bookmark.svg?raw'
import settingsIcon from '../assets/icons/settings.svg?raw'

const appStore = useAppStore()
const emit = defineEmits(['app-selected', 'download-started', 'accounts-updated', 'remove-item', 'clear-queue', 'logout'])
const isMobile = ref(false)
const subPage = ref(null) // null | 'appearance' | 'account' | 'changepassword'
const reloginEmail = ref('')

// email 有值时进入"重新登录"模式
function openAccountPage(email) {
  reloginEmail.value = typeof email === 'string' ? email : ''
  subPage.value = 'account'
}

const checkMobile = () => {
 isMobile.value = window.innerWidth < 768
}

const handleAccountsUpdated = (v) => emit('accounts-updated', v)
const handleAppSelected = (app) => emit('app-selected', app)
const handleDownloadStarted = (task) => emit('download-started', task)
const closeSubPage = () => {
  subPage.value = null
}
const selectTab = (tabId) => {
  closeSubPage()
  appStore.activeTab = tabId
}

const activeQueueCount = computed(() => appStore.taskQueue.filter(task => !['completed', 'ready'].includes(task?.status)).length)

const tabs = computed(() => [
 {
  id: 'download',
  label: '首页',
  icon: homeIcon,
  badge: appStore.downloadState.selectedApp ? '1' : null
 },
 {
  id: 'ipa',
  label: '队列',
  icon: clockIcon,
  badge: activeQueueCount.value > 0 ? String(activeQueueCount.value) : null
 },
 {
  id: 'archive',
  label: '收藏',
  icon: bookmarkIcon
 },
 {
  id: 'settings',
  label: '设置',
  icon: settingsIcon
 }
])

const currentTabComponent = computed(() => {
  const map = {
   download: DownloadManager,
   ipa: IpaManager,
   archive: ArchiveApp,
   settings: Settings
  }
  return map[appStore.activeTab] || DownloadManager
})

const currentTabProps = computed(() => {
  if (appStore.activeTab === 'download') {
   return {
     selectedApp: appStore.downloadState.selectedApp,
     accountsUpdated: appStore.accountsUpdateCounter
   }
  }
  if (appStore.activeTab === 'ipa') {
   return { queue: appStore.taskQueue }
  }
  return {}
})

// Warm async chunks in the background so first-time tab/sub-page navigation
// doesn't trigger a 1s white flash while the chunk is fetched.
function prefetchAsyncChunks() {
  const tasks = [
    () => import('./IpaManager.vue'),
    () => import('./ArchiveApp.vue'),
    () => import('./Settings.vue'),
    () => import('./Appearance.vue'),
    () => import('./AccountManager.vue')
  ]
  for (const task of tasks) {
    try {
      task().catch(() => {})
    } catch {
      // ignore — prefetch is best-effort
    }
  }
}

onMounted(() => {
  checkMobile()
  window.addEventListener('resize', checkMobile)

  if (typeof window !== 'undefined') {
    setTimeout(prefetchAsyncChunks, 300)
  }
})

onUnmounted(() => {
  window.removeEventListener('resize', checkMobile)
})

watch(() => appStore.activeTab, () => {
  closeSubPage()
})
</script>

<style scoped>
.tab-layout {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  height: 100%;
  min-height: 0;
  overflow: hidden;
}

.desktop-tabs {
  display: flex;
  gap: var(--space-2);
  border-bottom: 1px solid var(--color-border-light, #f0f0f0);
}

.desktop-tab {
  position: relative;
  display: inline-flex;
  align-items: center;
  gap: var(--space-2);
  height: 44px;
  padding: 0 var(--space-3);
  border: 0;
  border-radius: var(--radius-xl) var(--radius-xl) 0 0;
  background: transparent;
  color: var(--color-text-muted, #6e6e80);
  font-size: var(--font-size-body);
  font-weight: 500;
  cursor: pointer;
  transition: color 0.2s ease, background 0.2s ease;
}

.desktop-tab::after {
  content: '';
  position: absolute;
  left: 0;
  right: 0;
  bottom: -1px;
  height: 2px;
  border-radius: 9999px;
  background: transparent;
  transition: background 0.2s ease;
}

.desktop-tab.is-active {
  background: var(--color-primary-soft, #ecfdf5);
  color: var(--color-primary, #10a37f);
}

.desktop-tab.is-active::after {
  background: var(--color-primary, #10a37f);
}

.desktop-tab:hover:not(.is-active) {
  color: var(--color-text, #0d0d0d);
}

.tab-content {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

/* ===== Orbit v3 Mobile Tab Bar ===== */
.mobile-tabs-wrap {
  position: fixed;
  inset-inline: 0;
  bottom: 0;
  z-index: 50;
  background: var(--color-surface, #fff);
  padding-bottom: env(safe-area-inset-bottom);
  border-top: 1px solid var(--color-border-light, #f0f0f0);
}

.mobile-tabs {
  display: flex;
  align-items: flex-start;
  justify-content: space-around;
  height: 72px;
  padding: 10px 16px 0;
}

.mobile-tab {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 4px;
  min-width: 64px;
  border: 0;
  background: transparent;
  border-radius: 16px;
  cursor: pointer;
  padding: 6px 10px 8px;
  -webkit-tap-highlight-color: transparent;
  transition: background 0.2s ease, color 0.2s ease;
}

.mobile-tab.is-active {
  background: var(--color-primary-soft, #ecfdf5);
}

.mobile-tab__icon-wrap {
  position: relative;
  display: flex;
  align-items: center;
  justify-content: center;
  width: var(--size-tab-icon);
  height: var(--size-tab-icon);
}

.mobile-tab .tab-layout__icon {
  width: var(--size-tab-icon);
  height: var(--size-tab-icon);
  color: var(--color-text-muted, #6e6e80);
  transition: color 0.2s ease;
}

.mobile-tab.is-active .tab-layout__icon {
  color: var(--color-primary, #10a37f);
}

.mobile-tab__label {
  font-size: var(--font-size-micro);
  font-weight: 500;
  color: var(--color-text-muted, #6e6e80);
  line-height: 1.3;
  transition: color 0.2s ease;
}

.mobile-tab.is-active .mobile-tab__label {
  color: var(--color-primary, #10a37f);
  font-weight: 600;
}

.mobile-tab__badge {
  position: absolute;
  top: -4px;
  right: -10px;
  min-width: 18px;
  height: 18px;
  border-radius: 9999px;
  background: var(--color-danger);
  color: var(--color-text-inverse);
  font-size: var(--font-size-nano);
  font-weight: 600;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 0 4px;
  line-height: 1;
}

.tab-content.with-mobile-tabs {
  padding-bottom: calc(72px + env(safe-area-inset-bottom) + 8px);
}

/* ===== Dark Mode ===== */
.dark .desktop-tabs {
  border-bottom-color: var(--color-border, #3f3f46);
}

.dark .desktop-tab {
  color: var(--color-text-muted, #a1a1aa);
}

.dark .desktop-tab.is-active {
  background: rgba(52, 211, 153, 0.12);
  color: var(--color-primary, #34d399);
}

.dark .desktop-tab:hover:not(.is-active) {
  color: var(--color-text, #f5f5f5);
}

.dark .mobile-tabs-wrap {
  background: var(--color-surface, #18181b);
  border-top-color: var(--color-border, #3f3f46);
}

.dark .mobile-tab .tab-layout__icon {
  color: var(--color-text-muted, #a1a1aa);
}

.dark .mobile-tab.is-active .tab-layout__icon {
  color: var(--color-primary, #34d399);
}

.dark .mobile-tab.is-active {
  background: rgba(52, 211, 153, 0.12);
}

.dark .mobile-tab__label {
  color: var(--color-text-muted, #a1a1aa);
}

.dark .mobile-tab.is-active .mobile-tab__label {
  color: var(--color-primary, #34d399);
  font-weight: 600;
}

/* ===== Content hosts ===== */
/* Tab and settings sub-pages switch instantly to avoid mobile horizontal jitter. */
.subpage-host {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

/* tab-host: wrap so the inner KeepAlive layout is preserved */
.tab-host {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
}

</style>
