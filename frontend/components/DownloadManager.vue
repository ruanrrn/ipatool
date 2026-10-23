<template>
  <div class="home-page page-shell">
    <div class="home-page__fixed px-5">
      <h1 class="home-page__title">
        首页
      </h1>
      <div class="home-page__search-wrap">
        <!-- Search Bar -->
        <MobileInput
          v-model="searchQuery"
          placeholder="搜索应用名称或输入 App ID..."
          :loading="searching"
          :disabled="accounts.length === 0"
          clearable
          size="large"
          class="search-input search-input--fused-top"
          @input="handleSearch"
          @keyup.enter="handleSearch"
        >
          <template #prefix>
            <Search class="search-input__icon" />
          </template>
        </MobileInput>

        <AccountPicker
          :accounts="accounts"
          :selected-account="selectedAccount"
          @update:selected-account="selectedAccount = $event"
          @go-to-account="goToAccountTab"
        />

        <!-- Direct App ID Confirm Button -->
        <div
          v-if="isAppIdInput && searchQuery && !searching"
          class="status-panel mt-3 flex items-center justify-between p-4"
        >
          <div class="flex-1">
            <p class="text-caption font-medium text-txt-secondary">
              App ID: <span class="font-bold">{{ searchQuery.trim() }}</span>
            </p>
            <p class="text-nano text-txt-secondary mt-1">
              即使未找到应用信息，也可以继续查询版本号
            </p>
          </div>
          <MobileButton
            type="primary"
            size="small"
            @click="confirmDirectAppId"
          >
            确认并继续
          </MobileButton>
        </div>
      </div>
    </div>
    <div class="home-page__results">
      <div class="home-page__results-inner px-5">
        <!-- Search Results Count -->
        <p
          v-if="searchResults.length > 0"
          class="search-results-count"
        >
          找到 {{ searchResults.length }} 个结果
        </p>

        <!-- Search Results -->
        <div
          v-if="searchResults.length > 0"
          class="search-results-list"
        >
          <button
            v-for="app in searchResults"
            :key="app.trackId"
            class="result-item"
            @click="selectApp(app)"
          >
            <img 
              :src="app.artworkUrl100 || app.artworkUrl60"
              :alt="app.trackName"
              class="result-item__icon"
            >
            <div class="result-item__info">
              <h3 class="result-item__name">
                {{ app.trackName }}
              </h3>
              <p class="result-item__dev">
                {{ app.artistName }}
              </p>
              <div class="result-item__meta">
                <span class="result-item__tag">
                  {{ getSearchResultCategory(app) }}
                </span>
                <span class="result-item__tag">
                  {{ getSearchResultVersionLabel(app) }}
                </span>
                <span class="result-item__tag">
                  {{ getSearchResultSizeLabel(app) }}
                </span>
                <span class="result-item__tag result-item__tag--price">
                  {{ getSearchResultPriceLabel(app) }}
                </span>
                <span
                  class="result-item__tag"
                  :class="getSearchResultPurchaseClass(app)"
                >
                  {{ getSearchResultPurchaseLabel(app) }}
                </span>
              </div>
            </div>
            <span
              class="result-item__fav"
              :class="{ active: isAppFavorited(app.trackId) }"
              role="button"
              :aria-label="isAppFavorited(app.trackId) ? '取消收藏' : '收藏'"
              tabindex="0"
              @click.stop="quickToggleFavorite(app)"
              @keydown.enter.prevent="quickToggleFavorite(app)"
              @keydown.space.prevent="quickToggleFavorite(app)"
            >
              <i><component :is="isAppFavorited(app.trackId) ? StarFilled : Star" /></i>
              <span
                v-if="getArchivedVersionCount(app.trackId) > 0"
                class="result-item__fav-count"
              >
                {{ getArchivedVersionCount(app.trackId) }}
              </span>
            </span>
          </button>
        </div>

        <!-- Search Loading Skeleton -->
        <div
          v-if="searching && searchResults.length === 0"
          class="search-results-list search-results-list--skeleton"
          aria-busy="true"
          aria-label="搜索中"
        >
          <div
            v-for="n in 5"
            :key="`skeleton-${n}`"
            class="result-item result-item--skeleton"
          >
            <div class="skeleton skeleton--icon" />
            <div class="result-item__info result-item__info--skeleton">
              <div class="skeleton skeleton--line skeleton--title" />
              <div class="skeleton skeleton--line skeleton--sub" />
              <div class="skeleton skeleton--tags">
                <div class="skeleton skeleton--tag" />
                <div class="skeleton skeleton--tag" />
                <div class="skeleton skeleton--tag" />
              </div>
            </div>
          </div>
        </div>

        <!-- Search History (shown when no query, no results, no selected app) -->
        <div
          v-if="!searchQuery.trim() && searchResults.length === 0 && !selectedApp && searchHistory.length > 0"
          class="search-history"
        >
          <div class="search-history__header">
            <span class="search-history__title">最近搜索</span>
            <button
              class="search-history__clear"
              type="button"
              @click="clearSearchHistory"
            >
              清除
            </button>
          </div>
          <div class="search-history__tags">
            <button
              v-for="item in searchHistory"
              :key="`${item.region}:${item.query}:${item.ts}`"
              class="search-history__tag"
              type="button"
              @click="searchQuery = item.query; handleSearch()"
            >
              {{ item.query }}
            </button>
          </div>
        </div>

        <!-- Empty State -->
        <div
          v-if="!selectedApp && showHomeSections && activeTaskCount <= 0"
          class="empty-state-home"
        >
          <div class="empty-state-home__icon">
            📋
          </div>
          <p class="empty-state-home__title">
            暂无进行中的任务
          </p>
          <p class="empty-state-home__desc">
            搜索应用开始使用
          </p>
        </div>
      </div>
    </div>

    <DownloadVersionSheet
      v-model:version-note="versionNote"
      :app="selectedApp"
      :versions="versions"
      :selected-version="selectedVersion"
      :versions-fetched="versionsFetched"
      :fetching-versions="fetchingVersions"
      :appid="appid"
      :format-file-size="formatFileSize"
      :purchase-required="purchaseRequired"
      :download-blocked-reason="downloadBlockedReason"
      :current-version-download-url="currentVersionDownloadUrl"
      :current-version-install-url="currentVersionInstallUrl"
      :current-version-file-size="currentVersionFileSize"
      :current-version-ota-installable="currentVersionOtaInstallable"
      :current-version-install-method="currentVersionInstallMethod"
      :current-version-inspection="currentVersionInspection"
      :is-https="isHttps"
      :current-protocol="currentProtocol"
      :selected-account="selectedAccount"
      :checking-purchase-status="checkingPurchaseStatus"
      :claim-required="claimRequired"
      :claiming-selected-app="claimingSelectedApp"
      :purchase-action-label="purchaseActionLabel"
      :show-current-version-progress-card="showCurrentVersionProgressCard"
      :current-version-progress-mode="currentVersionProgressMode"
      :current-version-progress-button-label="currentVersionProgressButtonLabel"
      :download-blocked="downloadBlocked"
      :is-current-version-downloaded="isCurrentVersionDownloaded"
      :is-current-app-favorited="isCurrentAppFavorited"
      :favorite-loading="favoriteLoading"
      :downloading="downloading"
      @close="emit('app-selected', null)"
      @version-selected="handleVersionSelected"
      @manual-version-request="showManualVersionDialog = true"
      @download-ipa="downloadCompletedIpa"
      @install-ipa="installDownloadedIpa"
      @buy-or-claim="buyOrClaimSelectedApp"
      @download="directLinkDownload"
      @install="startInstallFlow"
      @toggle-favorite="toggleFavoriteApp"
    />

    <ManualVersionDialog
      v-model="showManualVersionDialog"
      @submit="handleManualVersionSubmit"
    />
  </div>
</template>

<script setup>
import { computed, ref, onMounted, onActivated, onBeforeUnmount, watch } from 'vue'

import { API_BASE } from '../config.js'
import { useAppStore } from '../stores/app'
import { useSearch } from '../composables/useSearch.js'
import { useAccounts } from '../composables/useAccounts.js'
import { Toast } from './MobileToast.vue'
import { Confirm } from './MobileConfirm.vue'
import { Search, Star, StarFilled } from './icons'
import { useDownload } from '../composables/useDownload.js'
import { apiFetch } from '../utils/api.js'
import { useCurrentVersionArtifacts } from '../composables/useCurrentVersionArtifacts.js'
import { useDownloadAccountSelection } from '../composables/useDownloadAccountSelection.js'
import { useDownloadArchiveFavorites } from '../composables/useDownloadArchiveFavorites.js'
import { useDownloadVersions } from '../composables/useDownloadVersions.js'
import { normalizeFetchedVersion } from '../utils/version.js'

import MobileButton from './MobileButton.vue'
import MobileInput from './MobileInput.vue'
import AccountPicker from './AccountPicker.vue'
import ManualVersionDialog from './ManualVersionDialog.vue'
import DownloadVersionSheet from './DownloadVersionSheet.vue'

const appStore = useAppStore()

const props = defineProps({
 selectedApp: {
 type: Object,
 default: null
 },
 accountsUpdated: {
 type: Number,
 default: 0
 }
})

// Make selectedApp reactive for template
const selectedApp = computed(() => props.selectedApp)

const emit = defineEmits(['download-started', 'app-selected'])

const { accounts, loadAccounts } = useAccounts()
const {
  selectedAccount,
  getRegionLabel,
  normalizeAccountIndex,
  resolveActiveAccount
} = useDownloadAccountSelection({ accounts, loadAccounts, appStore })

// Initialize download composable
const {
  downloading,
  showProgress,
  progressPercent,
  progressStage,
  showActionButtons,
  activeDownloadAppId,
  activeDownloadVersionId,
  activeDownloadAccountEmail,
  checkingPurchaseStatus,
  claimingSelectedApp,
  purchaseStatusText,
  purchaseRequired,
  claimRequired,
  downloadBlocked,
  downloadBlockedReason,
  purchaseActionLabel,
  isHttps,
  currentProtocol,
  startDownloadWithProgress,
  refreshPurchaseStatus,
  claimSelectedAppInBackground,
  addLog,
  localizeProgressStage,
  formatFileSize,
  openInstallUrl,
  openExternalUrl,
  getSelectedAppPrice
} = useDownload({
  onDownloadStarted: (queueItem) => {
    emit('download-started', queueItem)
    // Also sync to store
    appStore.addToQueue(queueItem)
  },
  onDownloadComplete: () => {
    // Load downloaded IPA files after completion
    loadDownloadedIpaFiles()
  },
  onDownloadFailed: (error) => {
    console.error('Download failed:', error)
  },
  selectedApp,
  selectedAccount,
  accounts
})

// Define helpers before useDownload since they're referenced in callbacks
const normalizeDownloadedIpaFile = (file) => ({
  ...file,
  appId: String(file?.appId ?? file?.app_id ?? file?.trackId ?? file?.id ?? ''),
  accountEmail: String(file?.accountEmail ?? file?.account_email ?? file?.apple_id ?? file?.email ?? ''),
  version: String(file?.version ?? file?.bundle_version ?? file?.appVersion ?? ''),
  versionId: String(file?.versionId ?? file?.version_id ?? file?.appVerId ?? file?.app_version_id ?? file?.external_identifier ?? '')
})

const loadDownloadedIpaFiles = async () => {
 try {
    const { data } = await apiFetch(`${API_BASE}/ipa-files`, { credentials: 'include' })
    if (data.ok && Array.isArray(data.data)) {
      downloadedIpaFiles.value = data.data.map(normalizeDownloadedIpaFile)
    }
  } catch (e) {
    console.warn('Failed to load ipa files:', e)
  }
}

const pendingAppStoreCheck = ref(false)
const downloadedIpaFiles = ref([])  // 缓存已下载的 IPA 文件列表

const {
 searchQuery,
 searchResults,
 searchResultPurchaseStatusMap,
 searching,
 isAppIdInput,
 searchHistory,
 clearSearchHistory,
 handleSearch,
 selectApp,
  confirmDirectAppId: rawConfirmDirectAppId,
 getSearchResultCategory,
 getSearchResultVersionLabel,
 getSearchResultSizeLabel,
 getSearchResultPriceLabel,
 getSearchResultPurchaseLabel,
 getSearchResultPurchaseClass,
} = useSearch(
 accounts,
 selectedAccount,
 (app) => emit('app-selected', app),
 { debounceDelay: 300 }
)

let syncSelectedVersionNote = () => {}

const {
  appid,
  appVerId,
  versions,
  selectedVersion,
  versionsFetched,
  fetchingVersions,
  showManualVersionDialog,
  debouncedFetchVersions,
  resetVersionStateForAppChange,
  confirmDirectAppId,
  handleVersionSelected,
  handleManualVersionSubmit
} = useDownloadVersions({
  accounts,
  selectedAccount,
  rawConfirmDirectAppId,
  getRegionLabel,
  addLog,
  syncSelectedVersionNote: (options) => syncSelectedVersionNote(options),
  Toast
})

const {
  favoriteLoading,
  versionNote,
  getArchivedVersionCount,
  isCurrentAppFavorited,
  isAppFavorited,
  loadArchivedAppIds,
  quickToggleFavorite,
  toggleFavoriteApp,
  syncSelectedVersionNote: syncArchiveSelectedVersionNote
} = useDownloadArchiveFavorites({
  accounts,
  selectedAccount,
  selectedApp,
  versions,
  selectedVersion,
  appVerId,
  Toast
})

syncSelectedVersionNote = syncArchiveSelectedVersionNote

const {
  showCurrentVersionProgressCard,
  currentVersionProgressMode,
  currentVersionDownloadUrl,
  currentVersionInstallUrl,
  currentVersionFileSize,
  currentVersionOtaInstallable,
  currentVersionInstallMethod,
  currentVersionInspection,
  currentVersionProgressButtonLabel,
  isCurrentVersionDownloaded,
  resolveSelectedVersionPayload
} = useCurrentVersionArtifacts({
  appid,
  appVerId,
  versions,
  selectedVersion,
  selectedApp,
  selectedAccount,
  accounts,
  downloadedIpaFiles,
  taskQueue: computed(() => appStore.taskQueue),
  localizeProgressStage
})

const activeTaskCount = computed(() => {
  return appStore.taskQueue.filter(t => t && ['downloading', 'processing'].includes(t.status)).length
})

const showHomeSections = computed(() => {
  return !searching.value && !searchQuery.value.trim() && searchResults.value.length === 0 && !selectedApp.value
})

// ===== End Orbit v3 =====

// isHttps, currentProtocol from useDownload (line 500-501)

// Sync state with store on mount and update
const syncStateToStore = () => {
 appStore.updateDownloadState('selectedAccountIndex', selectedAccount.value)
 // appid 由 setSelectedApp / watcher 管理，不再由此处同步，避免覆盖 setSelectedApp 刚写入的新值
 appStore.updateDownloadState('appVersionId', appVerId.value)
 appStore.updateDownloadState('availableVersions', versions.value)
 appStore.updateDownloadState('selectedVersionId', selectedVersion.value)
 appStore.updateDownloadState('versionsLoaded', versionsFetched.value)
 appStore.updateDownloadState('showProgressPanel', showProgress.value)
 appStore.updateDownloadState('progressPercentage', progressPercent.value)
 appStore.updateDownloadState('progressMessage', progressStage.value)
 appStore.updateDownloadState('activeDownloadAppId', activeDownloadAppId.value)
 appStore.updateDownloadState('activeDownloadVersionId', activeDownloadVersionId.value)
 appStore.updateDownloadState('activeDownloadAccountEmail', activeDownloadAccountEmail.value)
 appStore.updateDownloadState('searchQuery', searchQuery.value)
 appStore.updateDownloadState('searchResults', searchResults.value)
 appStore.updateDownloadState('searchResultPurchaseStatusMap', searchResultPurchaseStatusMap.value)
}

const restoreStateFromStore = () => {
 const state = appStore.downloadState
 // 只恢复非 undefined 的值，避免覆盖自动选择的账号
 if (state.selectedAccountIndex !== undefined && state.selectedAccountIndex !== null && state.selectedAccountIndex !== '') {
  selectedAccount.value = normalizeAccountIndex(state.selectedAccountIndex)
 }
 if (state.appId !== undefined) appid.value = state.appId
 if (state.appVersionId !== undefined) appVerId.value = state.appVersionId
 if (state.availableVersions !== undefined) {
  versions.value = (Array.isArray(state.availableVersions) ? state.availableVersions : [])
   .map(normalizeFetchedVersion)
   .filter(Boolean)
 }
 if (state.selectedVersionId !== undefined) selectedVersion.value = state.selectedVersionId
 if (state.versionsLoaded !== undefined) versionsFetched.value = state.versionsLoaded
 if (state.showProgressPanel !== undefined) showProgress.value = state.showProgressPanel
 if (state.progressPercentage !== undefined) progressPercent.value = state.progressPercentage
 if (state.progressMessage !== undefined) progressStage.value = localizeProgressStage(state.progressMessage)
 if (state.activeDownloadAppId !== undefined) activeDownloadAppId.value = state.activeDownloadAppId || ''
 if (state.activeDownloadVersionId !== undefined) activeDownloadVersionId.value = state.activeDownloadVersionId || ''
 if (state.activeDownloadAccountEmail !== undefined) activeDownloadAccountEmail.value = state.activeDownloadAccountEmail || ''
 downloading.value = !!(showProgress.value && !showActionButtons.value && progressPercent.value < 100)
 if (state.searchQuery !== undefined) searchQuery.value = state.searchQuery || ''
 if (state.searchResults !== undefined) searchResults.value = Array.isArray(state.searchResults) ? state.searchResults : []
 if (state.searchResultPurchaseStatusMap !== undefined) {
  searchResultPurchaseStatusMap.value = state.searchResultPurchaseStatusMap && typeof state.searchResultPurchaseStatusMap === 'object'
   ? state.searchResultPurchaseStatusMap
   : {}
 }
 // 如果没有 selectedApp prop，不恢复关联的 appid/versions 等状态
 if (!props.selectedApp) {
  appid.value = ''
  versions.value = []
  selectedVersion.value = ''
  versionsFetched.value = false
  appVerId.value = ''
 }
}

// 跳转到账号标签页
const goToAccountTab = () => {
 const appStore = useAppStore()
 appStore.activeTab = 'settings'
}

// Watch for selectedApp changes to auto-fill appid
watch(() => props.selectedApp, (newApp, oldApp) => {
  const newTrackId = newApp?.trackId ? String(newApp.trackId) : ''
  const oldTrackId = oldApp?.trackId ? String(oldApp.trackId) : ''

  // 只在真正切换应用（trackId 变化）时重置版本状态。
  // 这样 metadata enrich 二次 setSelectedApp 不会把已加载的版本列表清空。
  if (newTrackId !== oldTrackId) {
    resetVersionStateForAppChange({ loading: true })
    versionNote.value = ''
    loadArchivedAppIds()
  }

  appid.value = newTrackId
}, { immediate: true })

// Watch for account and appid changes to auto-fetch versions
watch([selectedAccount, appid], ([newAccount, newAppid]) => {
 if (newAccount !== '' && newAccount !== null && newAppid) {
 // 自动查询版本
 debouncedFetchVersions()
 }
})

watch(selectedVersion, () => {
 syncSelectedVersionNote({ force: true })
})

watch(
 [() => props.selectedApp?.trackId, selectedAccount],
 async ([trackId, accountIndex]) => {
 if (!trackId) {
 purchaseStatusText.value = '待检测'
 return
 }
 if (accountIndex === '' || accountIndex === null || accountIndex === undefined) {
 purchaseStatusText.value = '请选择账号后检测'
 return
 }
 await refreshSelectedAppMetadata()
 await refreshPurchaseStatus()
 },
 { immediate: true }
)

watch(
 [
  selectedAccount,
  appVerId,
  versions,
  selectedVersion,
  versionsFetched,
  showProgress,
  progressPercent,
  progressStage,
  activeDownloadAppId,
  activeDownloadVersionId,
  activeDownloadAccountEmail,
  searchQuery,
  searchResults,
  searchResultPurchaseStatusMap
 ],
 () => syncStateToStore(),
 { deep: true }
)

const refreshSelectedAppMetadata = async () => {
 if (!props.selectedApp?.trackId) return

const region = accounts.value[selectedAccount.value]?.region || 'US'
const needsFill = !props.selectedApp?.formattedPrice || !props.selectedApp?.fileSizeBytes
if (!needsFill) return

try {
const { data } = await apiFetch(`${API_BASE}/app-meta?appid=${encodeURIComponent(props.selectedApp.trackId)}&region=${encodeURIComponent(region)}`)
const app = data?.data
if (!data.ok || !app) return

appStore.setSelectedApp({
 ...props.selectedApp,
 ...app,
 })
 } catch (error) {
 console.warn('Failed to enrich selected app metadata:', error)
 }
}

const downloadCompletedIpa = () => {
  if (!currentVersionDownloadUrl.value) {
    Toast.warning('下载链接未显示')
    return
  }

  window.open(currentVersionDownloadUrl.value, '_blank', 'noopener')
}

// Wrapper functions that use composable functionality
const buyOrClaimSelectedApp = async () => {
  try {
    const price = getSelectedAppPrice()

    if (price === null) {
      Toast.show('价格未知，无法安全领取/购买。请先在搜索结果确认价格信息。')
      return
    }

    if (price <= 0) {
      const account = await resolveActiveAccount()
      if (!account) return
      await claimSelectedAppInBackground(appid.value, selectedVersion.value)
      return
    }

    const appStoreUrl = props.selectedApp?.trackViewUrl || `https://apps.apple.com/app/id${props.selectedApp.trackId}`
    pendingAppStoreCheck.value = true
    purchaseStatusText.value = '等待完成 App Store 购买后自动复检'
    Toast.info('即将打开 App Store 商品页。完成购买后返回此页，系统会自动重新检测状态。')
    openExternalUrl(appStoreUrl)
  } catch (error) {
    Toast.error(error.message || '操作失败')
  }
}

const installDownloadedIpa = async () => {
 if (!currentVersionInstallUrl.value) {
  Toast.warning('安装链接未生成')
  return
 }

 const isHttpsEnvironment = window.location.protocol === 'https:'
 const isLocalhost = window.location.hostname === 'localhost' || window.location.hostname === '127.0.0.1'

 if (!isHttpsEnvironment && !isLocalhost) {
  const action = await Confirm.show({ title: '无法开始 OTA 安装', message: '按 OpenList / Oplist 的现成方案，OTA 安装必须使用 HTTPS + 有效证书；当前环境不是 HTTPS，iOS 不会响应安装。您现在可以先直接下载 IPA，或改用 HTTPS 域名后再试。' }).then(
   () => 'download',
   () => 'cancel'
  ).catch(
   (action) => action === 'cancel' ? 'cancel' : 'close'
  )

  if (action === 'download') {
  downloadCompletedIpa()
  }
  return
 }

 if (isHttpsEnvironment) {
  Toast.success('正在打开安装链接...')
  openInstallUrl(currentVersionInstallUrl.value)
 } else if (isLocalhost) {
  const confirmed = await Confirm.show({ title: '安装前检查', message: '当前是 localhost 环境。按 OpenList / Oplist 文档，OTA 安装需要 HTTPS + 有效证书；localhost 基本不会成功。若你只是想继续试一把可以继续，否则请先切到 HTTPS 域名。' }).then(() => true).catch(() => false)

  if (confirmed) {
   openInstallUrl(currentVersionInstallUrl.value)
  }
 }
}

const startInstallFlow = async () => {
  try {
    const { versionId, versionLabel } = resolveSelectedVersionPayload()
    selectedVersion.value = versionId
    appVerId.value = versionId
    await startDownloadWithProgress(appid.value, versionId, false, true, versionLabel)
  } catch (error) {
    Toast.error(error.message || '安装启动失败')
  }
}

const directLinkDownload = async (autoPurchase = false) => {
  try {
    const { versionId, versionLabel } = resolveSelectedVersionPayload()
    selectedVersion.value = versionId
    appVerId.value = versionId
    await startDownloadWithProgress(appid.value, versionId, autoPurchase, false, versionLabel)
  } catch (error) {
    Toast.error(error.message || '下载启动失败')
  }
}

const handleAppStoreReturn = async () => {
 if (document.hidden || !pendingAppStoreCheck.value || checkingPurchaseStatus.value) return
 pendingAppStoreCheck.value = false

 // Use composable's refreshPurchaseStatus
 await refreshPurchaseStatus(appid.value, selectedVersion.value)
}

onMounted(() => {
 loadAccounts()
 loadArchivedAppIds()
 loadDownloadedIpaFiles()
 restoreStateFromStore()
 
 // 检测当前环境
 isHttps.value = window.location.protocol === 'https:'
 currentProtocol.value = window.location.protocol
 
 window.addEventListener('focus', handleAppStoreReturn)
 window.addEventListener('pageshow', handleAppStoreReturn)
 document.addEventListener('visibilitychange', handleAppStoreReturn)
 
 console.log(`[Environment] Protocol: ${currentProtocol.value}, HTTPS: ${isHttps.value}`)
})

onActivated(() => {
 loadArchivedAppIds()
 loadDownloadedIpaFiles()
})

onBeforeUnmount(() => {
 window.removeEventListener('focus', handleAppStoreReturn)
 window.removeEventListener('pageshow', handleAppStoreReturn)
 document.removeEventListener('visibilitychange', handleAppStoreReturn)
})
</script>

<style scoped>
/* ===== Orbit v3 Home Page ===== */

/* Page structure */
.home-page {
  flex: 1;
  min-height: 0;
  overflow: hidden;
  display: flex;
  flex-direction: column;
}

.home-page__fixed {
  flex-shrink: 0;
  padding-top: max(var(--space-5), env(safe-area-inset-top));
  background: var(--color-bg);
}

.home-page__results {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
}

.home-page__results-inner {
  padding-bottom: 24px;
}

.home-page__title {
 font-size: 26px;
 font-weight: 700;
 color: var(--color-text);
 line-height: 1.3;
 margin-bottom: 16px;
}

.home-page__stats {
 display: flex;
 gap: 10px;
 margin-bottom: 20px;
}

.home-page__search-wrap {
 margin-bottom: 20px;
}

/* Fused variants: search input top corners square */
.search-input--fused-top :deep(.mobile-input__wrapper) {
 border-radius: 14px 14px 0 0;
}

/* Search mode toggle */
.search-mode-row {
 display: flex;
 align-items: center;
 gap: 12px;
 flex-wrap: wrap;
 margin-bottom: 8px;
}

/* Search results count */
.search-results-count {
 font-size: 12px;
 font-weight: 500;
 color: var(--color-text-muted);
 margin-bottom: 8px;
}

/* Search results hint */
.search-results-hint {
 text-align: center;
 padding: 20px 0;
 font-size: 12px;
 color: var(--color-text-tertiary);
}

/* Search results list */
.search-results-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding-bottom: 4px;
  content-visibility: auto;
  contain-intrinsic-size: auto 96px;
}

/* Search result item card */
.result-item {
 display: flex;
 align-items: center;
 gap: 12px;
 padding: 12px;
 border-radius: 14px;
 background: var(--color-surface-muted);
 border: 1px solid var(--color-border);
 cursor: pointer;
 transition: background 0.2s ease;
 text-align: left;
 width: 100%;
 touch-action: manipulation;
 -webkit-tap-highlight-color: transparent;
 user-select: none;
 -webkit-user-select: none;
}

.result-item:active {
 background: var(--color-border-light);
}

/* ===== Skeleton loading ===== */
@keyframes skeleton-shimmer {
  0% { background-position: -200% 0; }
  100% { background-position: 200% 0; }
}
.result-item--skeleton {
  pointer-events: none;
}
.result-item--skeleton .skeleton,
.search-history .skeleton {
  background: linear-gradient(
    90deg,
    var(--color-surface-muted) 25%,
    var(--color-border-light) 50%,
    var(--color-surface-muted) 75%
  );
  background-size: 200% 100%;
  animation: skeleton-shimmer 1.5s ease-in-out infinite;
  border-radius: 6px;
}
.skeleton--icon {
  width: 52px;
  height: 52px;
  border-radius: 12px;
  flex-shrink: 0;
}
.result-item__info--skeleton {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.skeleton--line {
  height: 14px;
  border-radius: 4px;
}
.skeleton--title {
  width: 60%;
  height: 16px;
}
.skeleton--sub {
  width: 40%;
}
.skeleton--tags {
  display: flex;
  gap: 6px;
  margin-top: 4px;
}
.skeleton--tag {
  width: 52px;
  height: 18px;
  border-radius: 9px;
}

/* ===== Search history ===== */
.search-history {
  padding: 12px 0;
}
.search-history__header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 8px;
}
.search-history__title {
  font-size: 12px;
  font-weight: 600;
  color: var(--color-text-secondary);
}
.search-history__clear {
  font-size: 12px;
  color: var(--color-text-tertiary);
  background: none;
  border: none;
  cursor: pointer;
  padding: 4px 8px;
  border-radius: 4px;
  touch-action: manipulation;
  -webkit-tap-highlight-color: transparent;
  user-select: none;
  -webkit-user-select: none;
}
.search-history__clear:hover {
  color: var(--color-text-secondary);
  background: var(--color-surface-muted);
}
.search-history__tags {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}
.search-history__tag {
  font-size: 12px;
  padding: 6px 12px;
  border-radius: 16px;
  background: var(--color-surface-muted);
  border: 1px solid var(--color-border);
  color: var(--color-text-secondary);
  cursor: pointer;
  transition: background 0.15s ease;
  touch-action: manipulation;
  -webkit-tap-highlight-color: transparent;
  user-select: none;
  -webkit-user-select: none;
}
.search-history__tag:hover {
  background: var(--color-border-light);
}

.result-item__icon {
 width: 52px;
 height: 52px;
 border-radius: 12px;
 object-fit: cover;
 flex-shrink: 0;
}

.result-item__info {
 flex: 1;
 min-width: 0;
}

.result-item__name {
 font-size: 15px;
 font-weight: 600;
 color: var(--color-text);
 line-height: 1.3;
 overflow: hidden;
 text-overflow: ellipsis;
 white-space: nowrap;
}

.result-item__dev {
 font-size: 12px;
 color: var(--color-text-muted);
 margin-top: 1px;
 overflow: hidden;
 text-overflow: ellipsis;
 white-space: nowrap;
}

.result-item__meta {
 display: flex;
 flex-wrap: wrap;
 gap: 6px;
 margin-top: 4px;
}

.result-item__tag {
 font-size: 11px;
 padding: 2px 8px;
 border-radius: 4px;
 background: var(--color-bg-tag);
 color: var(--color-text-muted);
 line-height: 1.3;
}

.result-item__tag--success {
  background: var(--color-success-soft);
  color: var(--color-primary);
}

.result-item__tag--price {
  background: var(--color-blue-soft);
  color: var(--color-blue);
}

.result-item__tag--info {
  background: var(--color-blue-soft);
  color: var(--color-blue);
}

.result-item__tag--warning {
  background: var(--color-warning-soft);
  color: var(--color-warning-hover);
}

.result-item__tag--danger {
  background: var(--color-danger-soft);
  color: var(--color-danger-hover);
}

.result-item__tag--muted {
  background: var(--color-bg-tag);
  color: var(--color-text-muted);
}

.result-item__fav {
 flex-shrink: 0;
 min-width: 32px;
 height: 32px;
 display: inline-flex;
 align-items: center;
 justify-content: center;
 gap: 4px;
 padding: 0 8px;
 border: none;
 background: transparent;
 color: var(--color-text-tertiary);
 cursor: pointer;
 border-radius: 8px;
 transition: color 0.2s ease;
 touch-action: manipulation;
 -webkit-tap-highlight-color: transparent;
 user-select: none;
 -webkit-user-select: none;
 position: relative;
}

@media (pointer: coarse) {
  .result-item__fav::before {
    content: '';
    position: absolute;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    min-width: 44px;
    min-height: 44px;
    width: 100%;
    height: 100%;
  }
}

.result-item__fav-count {
 font-size: 11px;
 line-height: 1;
 font-weight: 700;
 min-width: 10px;
 text-align: center;
}

.result-item__fav.active {
 color: var(--color-warning);
}

.result-item__fav:hover {
 color: var(--color-warning);
}

/* Selected app card */
.selected-app-card {
 border-radius: 14px;
 background: var(--color-surface);
 border: 1px solid var(--color-border);
}

.selected-app-badges {
 display: flex;
 flex-wrap: wrap;
 gap: 8px;
}

.selected-app-badge,
.region-badge,
.region-badge-mini {
 display: inline-flex;
 align-items: center;
 padding: 4px 8px;
 border-radius: 6px;
 border: 1px solid var(--color-border);
 font-size: 11px;
 line-height: 1;
 color: var(--color-text-muted);
 background: transparent;
}

/* Action buttons */
.account-quick-select {
 width: 320px;
 max-width: 100%;
}

.account-option-row {
 display: flex;
 align-items: center;
 gap: 8px;
 width: 100%;
 min-width: 0;
}

.account-option-email {
 flex: 1;
 min-width: 0;
 overflow: hidden;
 text-overflow: ellipsis;
 white-space: nowrap;
}

/* Action buttons */
.action-button {
 border-radius: 8px;
 font-weight: 500;
 height: 44px;
}

.download-action-row {
 display: flex;
 gap: 8px;
 width: 100%;
}

.favorite-button {
 min-width: 100px;
}

.download-action-primary {
 flex: 1;
}

/* Empty state */
.empty-state-home {
 display: flex;
 flex-direction: column;
 align-items: center;
 justify-content: center;
 padding: 48px 24px;
 text-align: center;
 margin-top: 40px;
}

.empty-state-home__icon {
 font-size: 48px;
 margin-bottom: 16px;
 opacity: 0.4;
}

.empty-state-home__title {
 font-size: 16px;
 font-weight: 600;
 color: var(--color-text);
 margin-bottom: 6px;
}

.empty-state-home__desc {
 font-size: 13px;
 color: var(--color-text-muted);
 line-height: 1.5;
}

/* Search input overrides */
/* Search input focused state - matching design mockup */
.search-input :deep(.mobile-input__wrapper.is-focused) {
  border-color: var(--color-primary) !important;
  box-shadow: var(--shadow-search-focus);
}

/* Search icon sizing per design spec (18x18) */
.search-input :deep(.mobile-input__prefix svg),
.search-input :deep(.mobile-input__prefix i) {
 width: 18px;
 height: 18px;
}

/* Form select */
.form-select {
 border-radius: 10px;
}

/* Responsive */
@media (max-width: 767px) {
 .search-mode-row {
  gap: 12px;
 }

 .download-action-row {
  flex-direction: column;
 }

 .favorite-button,
 .download-action-primary {
  width: 100%;
 }

 .action-button {
  height: 44px;
 }
}

/* ===== Dark Mode ===== */
.dark .home-page__title {
 color: var(--color-text);
}

.dark .result-item {
 background: var(--color-surface);
 border-color: var(--color-surface-muted);
}

.dark .result-item:active {
 background: var(--color-surface-muted);
}

.dark .result-item__name {
 color: var(--color-text);
}

.dark .result-item__tag {
 background: var(--color-surface-muted);
 color: var(--color-text-muted);
}

.dark .selected-app-card {
 background: var(--color-surface);
 border-color: var(--color-surface-muted);
}

.dark .selected-app-badge,
.dark .region-badge,
.dark .region-badge-mini {
 border-color: var(--color-surface-muted);
 color: var(--color-text-muted);
}


.dark .result-item__fav {
 color: var(--color-text-tertiary);
}

.dark .result-item__fav.active,
.dark .result-item__fav:hover {
 color: var(--color-warning);
}

.dark .empty-state-home__title {
 color: var(--color-text);
}

.dark .empty-state-home__desc {
 color: var(--color-text-muted);
}

.dark .search-results-hint {
  color: var(--color-text-tertiary);
}

.dark .account-select-bar {
  background: var(--color-surface-muted);
  border-color: var(--color-surface-muted);
}

.dark .account-alert {
  background: var(--color-surface-muted);
  border-color: var(--color-surface-muted);
}

.dark .result-item__dev {
  color: var(--color-text-muted);
}

.dark .search-results-count {
  color: var(--color-text-muted);
}

.dark .account-alert .text-txt {
  color: var(--color-text);
}

.dark .account-alert .text-txt-secondary {
  color: var(--color-text-muted);
}

</style>

