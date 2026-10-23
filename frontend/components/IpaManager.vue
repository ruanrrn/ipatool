<template>
  <div class="queue-page space-y-0">
    <div class="queue-page__fixed px-5">
      <!-- Page Title + Segment Row -->
      <div class="queue-header">
        <div class="queue-header__title-wrap">
          <h1 class="page-title text-txt dark:text-txt-dark">
            队列
          </h1>
          <span class="queue-header__storage">占用 {{ formatFileSize(ipaStorageBytes) }}</span>
        </div>
        <div class="queue-header__right">
          <button
            v-if="artifacts.length > 0"
            class="q-btn q-btn--danger-text"
            @click="clearAllArtifacts"
          >
            一键清除
          </button>
        </div>
      </div>
      <div class="queue-segment">
        <button
          class="queue-seg"
          :class="{ active: activeTab === 'completed' }"
          @click="activeTab = 'completed'"
        >
          已完成 ({{ completedCount }})
        </button>
        <button
          class="queue-seg"
          :class="{ active: activeTab === 'active' }"
          @click="activeTab = 'active'"
        >
          活跃 ({{ activeTasks.length }})
        </button>
      </div>
    </div>

    <div class="queue-page__scroll">
      <div class="queue-page__scroll-inner px-5">
        <!-- Active Tab -->
        <IpaActiveTasksSection
          v-show="activeTab === 'active'"
          :tasks="activeTasks"
          :paused-task-ids="pausedTaskIds"
          @toggle-pause="togglePause"
          @remove-task="removeTask"
        />

        <!-- Completed Tab -->
        <IpaArtifactsSection
          v-show="activeTab === 'completed'"
          :artifacts="artifacts"
          @download="download"
          @install="install"
          @remove-artifact="removeArtifact"
        />
      </div>
    </div>

    <!-- Batch download queue: disabled until backend trigger is wired into UI
         (POST /batch-download has no frontend entry point). Kept for re-enablement.
    <div class="queue-page__batch px-5">
      <BatchQueue />
    </div>
    -->
  </div>
</template>

<script setup>
import { computed, onActivated, onMounted, reactive, ref, watch } from 'vue'
import { API_BASE } from '../config.js'

import { Toast } from './MobileToast.vue'
import { Confirm } from './MobileConfirm.vue'
import IpaActiveTasksSection from './IpaActiveTasksSection.vue'
import IpaArtifactsSection from './IpaArtifactsSection.vue'
import { useAppStore } from '../stores/app'
import { apiFetch } from '../utils/api.js'
import { useJobPolling } from '../composables/useJobPolling.js'


const props = defineProps({
  queue: { type: Array, default: () => [] }
})
const emit = defineEmits(['remove-item'])
const appStore = useAppStore()

// ── Tab State ──
const activeTab = computed({
  get: () => appStore.queueTab || 'completed',
  set: (value) => {
    appStore.queueTab = value
  }
})

// ── Pause state ──
const pausedTasks = reactive(new Set())
const pausedTaskIds = computed(() => Array.from(pausedTasks))

const togglePause = (taskId) => {
  if (pausedTasks.has(taskId)) {
    pausedTasks.delete(taskId)
  } else {
    pausedTasks.add(taskId)
  }
}

// ── Shared helpers ──

const download = (url) => window.open(url, '_blank', 'noopener')

const rewriteToCurrentOrigin = (rawUrl) => {
  const url = new URL(rawUrl, window.location.origin)
  url.protocol = window.location.protocol
  url.host = window.location.host
  return url.toString()
}

const buildInstallUrl = (installUrl) => {
  if (!installUrl) return null
  try {
    if (installUrl.startsWith('itms-services://')) {
      const itmsMatch = installUrl.match(/itms-services:\/\/\?action=download-manifest&url=(.+)/)
      if (!itmsMatch) return installUrl
      const manifestUrl = rewriteToCurrentOrigin(decodeURIComponent(itmsMatch[1]))
      return `itms-services://?action=download-manifest&url=${encodeURIComponent(manifestUrl)}`
    }
    const url = new URL(installUrl, window.location.origin)
    if (url.pathname === '/api/public/install' || url.pathname === '/api/install') {
      const manifest = url.searchParams.get('manifest')
      if (manifest) {
        const rewrittenManifest = rewriteToCurrentOrigin(manifest)
        return `itms-services://?action=download-manifest&url=${encodeURIComponent(rewrittenManifest)}`
      }
      return installUrl
    }
    return rewriteToCurrentOrigin(installUrl)
  } catch { return installUrl }
}

const install = (installUrl) => {
  const url = buildInstallUrl(installUrl)
  if (url) window.location.href = url
}

const formatFileSize = (bytes) => {
  if (!bytes) return '未知'
  const units = ['B', 'KB', 'MB', 'GB']
  let value = bytes
  let unitIndex = 0
  while (value >= 1024 && unitIndex < units.length - 1) { value /= 1024; unitIndex += 1 }
  return `${value.toFixed(value >= 100 || unitIndex === 0 ? 0 : 1)} ${units[unitIndex]}`
}

// ── Active download tasks ──

// Only show tasks that are NOT in a final state
const activeTasks = computed(() => props.queue.filter(task => ['downloading', 'processing', 'queued', 'waiting', 'running'].includes(task?.status)))

const completedCount = computed(() => artifacts.value.length)

// Initialize job polling composable
const { syncPolling, stopPolling } = useJobPolling({
  isFinalStatus: (status) => ['completed', 'ready', 'failed', 'error'].includes(status),
  pollInterval: 1500,
  maxFailures: 5,
  onUpdate: (taskId, snapshot) => {
    // Normalize server status to client status
    // Server uses: 'queued', 'running', 'ready', 'failed'
    // Client expects: 'queued', 'downloading', 'processing', 'completed', 'failed'
    const serverToClientStatus = {
      'running': 'downloading',
      'active': 'downloading',
    }
    const normalizedStatus = serverToClientStatus[snapshot.status]
      || (snapshot.status === 'ready' ? 'completed' : snapshot.status)

    // Sync task snapshot
    const updates = {
      stage: snapshot.stage || '',
      progress: snapshot.progress ?? 0,
      error: snapshot.error || '',
      status: normalizedStatus,
      packageKind: snapshot.packageKind,
      otaInstallable: snapshot.otaInstallable,
      installMethod: snapshot.installMethod,
      inspection: snapshot.inspection
    }
    appStore.updateQueueItem(taskId, updates)
  },
  onComplete: async (taskId, snapshot) => {
    // Handle completion
    const updates = {
      status: 'completed',
      stage: snapshot.stage || 'done',
      progress: 100,
      downloadUrl: snapshot.downloadUrl,
      installUrl: snapshot.installUrl,
      fileSize: snapshot.fileSize || 0,
      packageKind: snapshot.packageKind,
      otaInstallable: snapshot.otaInstallable,
      installMethod: snapshot.installMethod,
      inspection: snapshot.inspection
    }
    appStore.updateQueueItem(taskId, updates)
    // Refresh IPA list when download completes
    await loadArtifacts()
    // Auto-remove from queue after a short delay
    setTimeout(() => {
      appStore.removeFromQueue(taskId)
    }, 1500)
  },
  onFailed: (taskId, snapshot) => {
    // Handle failure - polling already stopped by composable
    // Normalize status: ensure it's 'failed' regardless of server value
    appStore.updateQueueItem(taskId, {
      stage: snapshot.stage || '',
      error: snapshot.error || '任务失败',
      status: snapshot.status === 'ready' ? 'completed' : (snapshot.status === 'failed' || snapshot.status === 'error' ? snapshot.status : 'failed')
    })
  },
  onInterrupted: (taskId, message) => {
    // Handle interrupted task
    appStore.updateQueueItem(taskId, {
      status: 'failed',
      stage: 'interrupted',
      error: message
    })
  },
  onError: (taskId, message, failureCount) => {
    // Handle polling errors (non-fatal)
    console.warn(`Polling error for task ${taskId} (${failureCount}/${5}): ${message}`)
  }
})

// Helper to check if task is in final state
const isFinalStatus = (status) => ['completed', 'ready', 'failed', 'error'].includes(status)

const removeTask = async (id) => {
  const task = props.queue.find(t => t?.id === id)
  if (!task) { stopPolling(id); emit('remove-item', id); return }
  const isActive = !isFinalStatus(task.status)
  if (isActive) {
    const confirmed = await Confirm.show({
      title: '确认取消任务',
      message: `任务「${task.appName || '未知'}」仍在进行中，确定取消吗？`,
      confirmText: '取消任务',
      cancelText: '继续等待',
      type: 'danger'
    })
    if (!confirmed) return
  }
  stopPolling(id)
  emit('remove-item', id)
}

// ── IPA artifacts ──

const artifacts = ref([])
const ipaLoading = ref(false)
const deletingArtifact = ref(false)

const ipaStorageBytes = computed(() => artifacts.value.reduce((sum, item) => sum + Number(item.fileSize || 0), 0))

const loadArtifacts = async () => {
  ipaLoading.value = true
  try {
    const { data } = await apiFetch(`${API_BASE}/ipa-files`)
    if (!data.ok) throw new Error(data.error || '加载失败')
    artifacts.value = data.data || []
  } catch (error) {
    Toast.error(error.message || '加载失败')
  } finally {
    ipaLoading.value = false
  }
}

const deleteArtifactById = async (id) => {
  const { response, data } = await apiFetch(`${API_BASE}/ipa-files/${id}`, { method: 'DELETE' })
  if (data.ok) return { missing: false }
  if (response.status === 404 || data.error === 'IPA 文件不存在') return { missing: true }
  throw new Error(data.error || '删除失败')
}

const removeArtifact = async (item) => {
  if (deletingArtifact.value) return

  const versionLabel = item?.version ? `v${item.version}` : ''
  const confirmed = await Confirm.show({
    title: '删除 IPA 文件',
    message: `确定删除「${item?.appName || '未知应用'}」${versionLabel}吗？此操作不可恢复。`
  })
  if (!confirmed) return

  deletingArtifact.value = true
  try {
    const result = await deleteArtifactById(item.id)
    if (result.missing) {
      Toast.warning('文件已不存在，列表已刷新')
    } else {
      Toast.success('IPA 已删除')
    }
    await loadArtifacts()
  } catch (error) {
    Toast.error(error.message || '删除失败')
  } finally {
    deletingArtifact.value = false
  }
}

const clearAllArtifacts = async () => {
  if (artifacts.value.length === 0) return
  const totalCount = artifacts.value.length
  const totalSizeLabel = formatFileSize(ipaStorageBytes.value)
  const confirmed = await Confirm.show({
    title: '确认一键清除',
    message: `确定清除全部 ${totalCount} 个安装包吗？当前共占用 ${totalSizeLabel}`,
    confirmText: '全部清除',
    cancelText: '取消',
    type: 'danger'
  })
  if (!confirmed) return

  try {
    for (const item of [...artifacts.value]) {
      await deleteArtifactById(item.id)
    }
    Toast.success(`已清除全部安装包（${totalCount} 个）`)
    await loadArtifacts()
  } catch (error) {
    Toast.error(error?.message || '一键清除失败')
  }
}


// ── Lifecycle ──

watch(
  () => props.queue.map(task => `${task?.id}:${task?.status}:${task?.progress}:${task?.stage}`),
  () => { syncPolling(props.queue) },
  { immediate: true }
)

onMounted(() => {
  loadArtifacts()
  syncPolling(props.queue)
})

onActivated(() => {
  loadArtifacts()
})
</script>

<style scoped>
.queue-page {
  flex: 1;
  min-height: 0;
  overflow: hidden;
  display: flex;
  flex-direction: column;
}

.queue-page__fixed {
  flex-shrink: 0;
  padding-top: max(var(--space-5), env(safe-area-inset-top));
  background: var(--color-bg);
}

.queue-page__scroll {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
}

.queue-page__scroll-inner {
  padding-bottom: 24px;
}

/* Page title + header row */
.queue-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  min-height: 34px;
  margin-bottom: 16px;
}
.queue-header__title-wrap {
  display: flex;
  align-items: baseline;
  gap: 8px;
  min-width: 0;
}
.queue-header__storage {
  font-size: var(--font-size-caption);
  color: var(--color-text-muted, #6e6e80);
  white-space: nowrap;
}
.queue-header__right {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
}

.page-title {
  font-size: var(--font-size-title);
  font-weight: 700;
  line-height: 1.3;
  margin-bottom: 0;
}

/* Segment Control */
.queue-segment {
 display: flex;
 gap: 0;
 background: var(--color-surface-muted, #f7f7f8);
 border-radius: var(--radius-xl);
 padding: 3px;
 margin-bottom: 0;
}
.dark .queue-segment {
 background: var(--color-surface, #18181b);
}

.queue-seg {
 flex: 1;
 padding: 9px;
 text-align: center;
 font-size: var(--font-size-label);
 font-weight: 500;
 border-radius: var(--radius-lg);
 color: var(--color-text-muted, #6e6e80);
 border: none;
 background: transparent;
 cursor: pointer;
 transition: all 0.2s ease;
 -webkit-tap-highlight-color: transparent;
 touch-action: manipulation;
 user-select: none;
 -webkit-user-select: none;
}
.dark .queue-seg {
 color: var(--color-text-muted, #a1a1aa);
}

.queue-seg.active {
 background: var(--color-surface, #fff);
 color: var(--color-text, #0d0d0d);
 box-shadow: 0 1px 3px rgba(0, 0, 0, 0.08);
}
.dark .queue-seg.active {
 background: var(--color-surface-muted, #27272a);
 color: var(--color-text, #f5f5f5);
 box-shadow: 0 1px 3px rgba(0, 0, 0, 0.3);
}

.dark .page-title {
  color: var(--color-text, #f5f5f5);
}
.dark .queue-header__storage {
  color: var(--color-text-muted, #a1a1aa);
}

/* Action button base */
.q-btn {
 width: 32px;
 height: 32px;
 border-radius: var(--radius-base);
 border: 1px solid var(--color-border, #ebebeb);
 background: var(--color-surface, #fff);
 color: var(--color-text-muted, #6e6e80);
 font-size: var(--font-size-body);
 display: flex;
 align-items: center;
 justify-content: center;
 cursor: pointer;
 transition: all 0.2s ease;
 -webkit-tap-highlight-color: transparent;
 padding: 0;
}
.dark .q-btn {
 background: var(--color-surface, #18181b);
 border-color: var(--color-surface-muted, #27272a);
 color: var(--color-text-muted, #a1a1aa);
}
.q-btn:active {
  opacity: 0.7;
}

/* Text-style danger button for header actions */
.q-btn--danger-text {
  width: auto;
  height: auto;
  padding: 4px 10px;
  border-radius: var(--radius-base);
  border: 1px solid var(--color-danger-border);
  background: transparent;
  color: var(--color-danger);
  font-size: var(--font-size-caption);
  font-weight: 500;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  transition: all 0.2s ease;
  -webkit-tap-highlight-color: transparent;
  white-space: nowrap;
}
.dark .q-btn--danger-text {
  color: var(--color-danger);
  border-color: rgba(239, 68, 68, 0.3);
}
.q-btn--danger-text:active {
 opacity: 0.7;
}

/* Batch queue section spacing */
.queue-page__batch {
  padding-top: 16px;
  padding-bottom: 24px;
  border-top: 1px solid var(--color-border, #ebebeb);
  margin-top: 8px;
}
.dark .queue-page__batch {
  border-color: var(--color-border, #27272a);
}
</style>
