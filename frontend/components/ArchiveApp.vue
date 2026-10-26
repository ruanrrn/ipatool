<template>
  <div class="archive-page space-y-0">
    <div class="archive-page__fixed px-5">
      <!-- Page Title -->
      <h1 class="page-title text-txt dark:text-txt-dark">
        收藏
      </h1>

      <ArchiveSegmentTabs
        v-model="activeTab"
        :favorites-count="favoriteVersionItems.length"
        :delisted-count="delistedApps.length"
      />
    </div>

    <div class="archive-page__scroll">
      <div class="archive-page__scroll-inner px-5">
        <div
          v-show="activeTab === 'favorites'"
          class="archive-panel"
        >
          <div
            v-if="favoritesLoading"
            class="archive-empty archive-empty--loading"
          >
            <EmptyState
              type="loading"
              text=""
            />
          </div>
          <div
            v-else-if="favorites.length === 0"
            class="archive-empty"
          >
            <EmptyState
              type="empty"
              text="暂无收藏"
            />
          </div>
          <div
            v-else
            class="fav-list"
          >
            <ArchiveFavoriteVersionItem
              v-for="item in favoriteVersionItems"
              :key="`fav-${item.appId}-${item.version_id || item.version || 'default'}`"
              :item="item"
              :downloading="downloadingAppId === (item.archive_key || item.appId)"
              @open="prepareApp"
              @download="downloadArchivedVersion"
              @remove="removeFavoriteVersion"
            />
          </div>
        </div>

        <div
          v-show="activeTab === 'delisted'"
          class="archive-panel"
        >
          <div
            v-if="integrityWarnings.length"
            class="integrity-warnings"
          >
            <div
              v-for="w in integrityWarnings"
              :key="w.appId"
              class="integrity-warning"
            >
              ⚠️ {{ w.name }}：{{ w.message }}
            </div>
          </div>

          <div class="archive-section">
            <div class="archive-section__header">
              <div>
                <div class="archive-section__title">
                  社区归档
                </div>
                <div class="archive-section__desc">
                  读取官方下架索引，可直接查看和下载已公开归档版本
                </div>
              </div>
              <div class="archive-section__meta">
                {{ delistedApps.length }} 项
              </div>
            </div>

            <div
              v-if="delistedLoading"
              class="archive-empty archive-empty--loading"
            >
              <EmptyState
                type="loading"
                text=""
              />
            </div>
            <div
              v-else-if="delistedApps.length === 0"
              class="archive-empty"
            >
              <EmptyState
                type="empty"
                text="暂无社区下架归档"
              />
            </div>
            <div
              v-else
              class="fav-list"
            >
              <ArchiveAppCard
                v-for="app in delistedApps"
                :key="`delisted-${app.archive_key || app.id}`"
                :app="app"
                :selected-version="getSelectedVersion(app)"
                :busy="downloadingAppId === (app.archive_key || app.id)"
                action="download"
                @open="prepareCommunityApp"
                @download="openArchiveVersionSelect"
              />
            </div>
          </div>

          <div class="archive-section archive-section--contribution">
            <div class="archive-section__header">
              <div>
                <div class="archive-section__title">
                  本地待贡献
                </div>
                <div class="archive-section__desc">
                  从本地下载记录聚合，自动过滤社区已收录条目；配置 GitHub PAT 后可贡献
                </div>
              </div>
              <div class="archive-section__meta">
                {{ localCandidates.length }} 项
              </div>
            </div>

            <div
              v-if="localCandidatesLoading"
              class="archive-empty archive-empty--loading"
            >
              <EmptyState
                type="loading"
                text=""
              />
            </div>
            <div
              v-else-if="localCandidates.length === 0"
              class="archive-empty"
            >
              <EmptyState
                type="empty"
                text="暂无本地待贡献应用"
              />
            </div>
            <div
              v-else
              class="fav-list"
            >
              <ArchiveAppCard
                v-for="app in localCandidates"
                :key="`candidate-${app.archive_key || app.id}`"
                :app="app"
                :selected-version="getSelectedVersion(app)"
                :busy="contributingAppId === (app.archive_key || app.id)"
                :action="githubTokenConfigured ? 'publish' : 'none'"
                tag="待贡献"
                @open="githubTokenConfigured ? prepareCandidateContribution($event) : null"
                @publish="prepareCandidateContribution"
              />
            </div>
          </div>
        </div>

        <div class="fav-hint">
          点击 ★ 可取消收藏 · 同一应用可收藏多个版本
        </div>
      </div>
    </div>

    <ArchiveVersionSelectSheet
      v-model="versionSelectSheet.visible"
      :app="versionSelectSheet.app"
      :versions="versionSelectSheet.versions"
      :selected-version-id="versionSelectSheet.selectedVersionId"
      :loading="versionSelectSheet.loading"
      @select="handleArchiveVersionSelect"
      @download="downloadSelectedArchiveVersion"
    />

    <ArchivePublishDialog
      v-model:visible="publishDialog.visible"
      v-model:notes="publishDialog.notes"
      :app-name="publishDialog.appName"
      :warnings="publishDialog.warnings"
      :loading="publishDialog.loading"
      :result="publishDialog.result"
      @publish="doPublish"
    />
  </div>
</template>

<script setup>
import { computed, onActivated, onMounted, reactive } from 'vue'

import EmptyState from './EmptyState.vue'
import { Toast } from './MobileToast.vue'
import { useAppStore } from '../stores/app'
import ArchiveVersionSelectSheet from './ArchiveVersionSelectSheet.vue'
import ArchiveFavoriteVersionItem from './ArchiveFavoriteVersionItem.vue'
import ArchiveAppCard from './ArchiveAppCard.vue'
import ArchiveSegmentTabs from './ArchiveSegmentTabs.vue'
import ArchivePublishDialog from './ArchivePublishDialog.vue'
import { useArchiveAccounts } from '../composables/useArchiveAccounts.js'
import { useArchiveData } from '../composables/useArchiveData.js'
import { useArchiveDownload } from '../composables/useArchiveDownload.js'
import { useArchivePublish } from '../composables/useArchivePublish.js'
import { useArchiveVersions } from '../composables/useArchiveVersions.js'

const appStore = useAppStore()

const activeTab = computed({
  get: () => appStore.archiveTab || 'favorites',
  set: (value) => {
    appStore.archiveTab = value
  }
})

const {
  activeAccount,
  ensureAccounts,
  requireActiveAccount
} = useArchiveAccounts()
const {
  selectedVersionByApp,
  getVersionOptions,
  getSelectedVersion,
  setSelectedVersion,
  applyVersionDefaults,
  prepareVersions
} = useArchiveVersions({ activeAccount })
const {
  favorites,
  delistedApps,
  localCandidates,
  remoteDelistedIds,
  favoritesLoading,
  delistedLoading,
  localCandidatesLoading,
  integrityWarnings,
  favoriteVersionItems,
  refreshAll,
  removeFavoriteVersion,
  prepareCommunityApp
} = useArchiveData({ ensureAccounts, applyVersionDefaults, prepareVersions })

const versionSelectSheet = reactive({
  visible: false,
  app: null,
  versions: [],
  selectedVersionId: '',
  loading: false
})

const githubTokenConfigured = computed(() => appStore.githubTokenStatus.configured)

const prepareApp = async (app) => {
  try {
    const versions = await prepareVersions(app)
    if (!versions.length && !getVersionOptions(app).length) {
      Toast.warning('未获取到可用版本')
    }
  } catch (error) {
    if (!getVersionOptions(app).length) {
      Toast.warning(error.message || '加载版本失败')
    }
  }
}

const openArchiveVersionSelect = async (app) => {
  versionSelectSheet.visible = true
  versionSelectSheet.app = app
  versionSelectSheet.loading = true

  const archiveKey = app.archive_key || app.id
  try {
    await prepareCommunityApp(app)
    const options = getVersionOptions(app)
    versionSelectSheet.versions = options

    const selectedVersion = selectedVersionByApp.value[archiveKey] || options[0]?.version_id || ''
    versionSelectSheet.selectedVersionId = selectedVersion
    if (selectedVersion) setSelectedVersion(archiveKey, selectedVersion)
  } catch (error) {
    versionSelectSheet.versions = []
    Toast.warning(error.message || '加载归档版本失败')
  } finally {
    versionSelectSheet.loading = false
  }
}

const handleArchiveVersionSelect = (versionId) => {
  if (!versionSelectSheet.app) return
  const archiveKey = versionSelectSheet.app.archive_key || versionSelectSheet.app.id
  versionSelectSheet.selectedVersionId = versionId
  setSelectedVersion(archiveKey, versionId)
}

const {
  downloadingAppId,
  downloadArchivedApp,
  downloadArchivedVersion
} = useArchiveDownload({
  appStore,
  requireActiveAccount,
  selectedVersionByApp,
  getVersionOptions,
  prepareApp,
  prepareCommunityApp
})

const downloadSelectedArchiveVersion = async () => {
  if (!versionSelectSheet.app) return
  await downloadArchivedApp(versionSelectSheet.app, versionSelectSheet.selectedVersionId, {
    onSuccess: () => {
      versionSelectSheet.visible = false
    }
  })
}

const {
  contributingAppId,
  publishDialog,
  prepareCandidateContribution,
  doPublish
} = useArchivePublish({ githubTokenConfigured, remoteDelistedIds })

const refreshArchiveView = async () => {
  await Promise.all([
    refreshAll(),
    appStore.loadGithubTokenStatus().catch((error) => {
      console.warn('[ArchiveApp] loadGithubTokenStatus failed:', error.message)
    })
  ])
}

onMounted(refreshArchiveView)
onActivated(refreshArchiveView)
</script>

<style scoped>
.archive-page {
  flex: 1;
  min-height: 0;
  overflow: hidden;
  display: flex;
  flex-direction: column;
  font-size: var(--font-size-md);
}

.archive-page__fixed {
  flex-shrink: 0;
  padding-top: max(var(--space-5), env(safe-area-inset-top));
  background: var(--color-bg);
}

.archive-page__scroll {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
}

.archive-page__scroll-inner {
  padding-bottom: 24px;
}

.archive-panel {
  min-height: 100%;
  display: flex;
  flex-direction: column;
}

/* Page title */
.page-title {
  font-size: var(--font-size-title);
  font-weight: 700;
  line-height: 1.3;
  margin-bottom: 16px;
}

/* Toolbar */
.fav-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  margin-bottom: 16px;
  flex-wrap: wrap;
}

.fav-account-select {
  width: 200px;
  max-width: 100%;
}

/* Archive section layout */
.fav-list {
  padding-top: 8px;
  content-visibility: auto;
  contain-intrinsic-size: auto 88px;
}

.archive-section {
  margin-top: 16px;
}

.archive-section--contribution {
  margin-top: 22px;
}

.archive-section__header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: nowrap;
}

.archive-section__header > div:first-child {
  min-width: 0;
  flex: 1;
  max-width: calc(100% - 56px);
}

.archive-section__meta {
  flex-shrink: 0;
  white-space: nowrap;
}

.archive-section__title {
  font-size: var(--font-size-body);
  font-weight: 700;
  color: var(--color-text);
}

.archive-section__desc,
.archive-section__meta {
  font-size: var(--font-size-caption);
  color: var(--color-text-muted);
}

.archive-section__desc {
  margin-top: 4px;
}

.dark .archive-section__title {
  color: var(--color-text);
}

.dark .archive-section__desc,
.dark .archive-section__meta {
  color: var(--color-text-muted);
}

/* Empty state */
.archive-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  flex: 1;
  min-height: 220px;
  padding: 8px 0 16px;
}

/* Bottom hint */
.fav-hint {
 font-size: 11px;
 color: var(--color-text-tertiary);
 text-align: center;
 padding: 12px 0;
}
.dark .fav-hint {
 color: var(--color-text-tertiary);
}

/* Integrity warnings */
.integrity-warnings {
  margin-bottom: 12px;
}
.integrity-warning {
  font-size: 12px;
  color: var(--color-warning);
  background: var(--color-warning-soft);
  border: 1px solid var(--color-warning-border);
  border-radius: 8px;
  padding: 8px 12px;
  margin-bottom: 6px;
  line-height: 1.4;
}
.dark .integrity-warning {
  color: var(--color-amber-400);
  background: rgba(var(--rgb-amber), 0.12);
  border-color: rgba(var(--rgb-amber), 0.25);
}
.dark .archive-empty {
 color: var(--color-text-muted);
}

/* Spin animation */
@keyframes spin {
  from { transform: rotate(0deg); }
  to { transform: rotate(360deg); }
}
.animate-spin {
  animation: spin 1s linear infinite;
}

@media (max-width: 767px) {
  .fav-account-select {
    width: 100%;
  }
}
</style>
