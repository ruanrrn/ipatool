import { ref, computed, onBeforeUnmount } from 'vue'
import { API_BASE } from '../config.js'
import { apiFetch } from '../utils/api.js'

/**
 * 批量下载任务的队列管理 composable。
 *
 * 后端契约（server/src/database.rs）：
 *   BatchDownloadTask: { id, task_name, status, total_count, completed_count,
 *                        failed_count, created_at, updated_at, completed_at }
 *   BatchDownloadItem: { id, batch_id, app_id, app_name, version, account_email,
 *                        status, progress, error, retry_count, created_at }
 *
 * Task status:  "pending" | "processing" | "completed" | "failed"
 * Item status:  "pending" | "completed" | "failed"
 *
 * 该 composable 负责：
 *   - 拉取批量任务列表 + 每个 task 展开后的 items
 *   - 轮询进行中的任务（status=pending/processing）以更新进度
 *   - 删除任务
 */

const POLL_INTERVAL_MS = 3000

export function useBatchTasks() {
  const tasks = ref([])
  const expandedTaskIds = ref(new Set())
  const itemCache = ref({}) // { [batchId]: items[] }
  const loading = ref(false)
  const error = ref(null)
  let pollTimer = null
  let polling = false
  let disposed = false
  let generation = 0
  let listRequest = null
  const controllers = new Set()
  const itemRequests = new Map()
  const request = async (url, options = {}) => {
    const controller = new AbortController()
    controllers.add(controller)
    try {
      return await apiFetch(url, { ...options, signal: controller.signal })
    } finally {
      controllers.delete(controller)
    }
  }

  const activeTasks = computed(() =>
    tasks.value.filter(t => ['pending', 'queued', 'processing', 'running'].includes(t.status))
  )
  const completedTasks = computed(() =>
    tasks.value.filter(t => t.status === 'completed')
  )
  const hasAnyTasks = computed(() => tasks.value.length > 0)

  /** 拉取所有批量任务列表 */
  function fetchTasks() {
    if (disposed) return Promise.resolve()
    if (listRequest) return listRequest
    const epoch = generation
    loading.value = true
    error.value = null
    const pending = (async () => {
      try {
        const { data } = await request(`${API_BASE}/batch-tasks`)
        if (disposed || epoch !== generation) return
        if (data?.ok && Array.isArray(data.data)) {
          tasks.value = data.data
          await Promise.all([...expandedTaskIds.value].map(fetchItems))
        }
      } catch (e) {
        if (!disposed && epoch === generation) error.value = e?.message || '获取批量任务失败'
      } finally {
        if (!disposed && epoch === generation) loading.value = false
      }
    })()
    listRequest = pending
    void pending.finally(() => {
      if (listRequest === pending) listRequest = null
    })
    return pending
  }

  /** Deduplicate expanded item requests and discard responses after cleanup. */
  function fetchItems(batchId) {
    if (disposed) return Promise.resolve()
    if (itemRequests.has(batchId)) return itemRequests.get(batchId)
    const epoch = generation
    const pending = (async () => {
      try {
        const { data } = await request(`${API_BASE}/batch-tasks/${batchId}`)
        if (!disposed && epoch === generation && data?.ok && data.data?.items) {
          itemCache.value = { ...itemCache.value, [batchId]: data.data.items }
        }
      } catch {
        // Retry on the next refresh.
      }
    })()
    itemRequests.set(batchId, pending)
    void pending.finally(() => {
      if (itemRequests.get(batchId) === pending) itemRequests.delete(batchId)
    })
    return pending
  }

  /** 展开/收起任务详情 */
  async function toggleExpand(task) {
    const id = task.id
    if (!id) return
    if (expandedTaskIds.value.has(id)) {
      expandedTaskIds.value.delete(id)
      expandedTaskIds.value = new Set(expandedTaskIds.value)
    } else {
      expandedTaskIds.value.add(id)
      expandedTaskIds.value = new Set(expandedTaskIds.value)
      await fetchItems(id)
    }
  }

  /** 删除任务 */
  async function deleteTask(batchId) {
    try {
      const epoch = generation
      const { data } = await request(`${API_BASE}/batch-tasks/${batchId}`, {
        method: 'DELETE',
      })
      if (!disposed && epoch === generation && data?.ok) {
        tasks.value = tasks.value.filter(t => t.id !== batchId)
        const next = { ...itemCache.value }
        delete next[batchId]
        itemCache.value = next
        expandedTaskIds.value.delete(batchId)
        expandedTaskIds.value = new Set(expandedTaskIds.value)
        return true
      }
      return false
    } catch {
      return false
    }
  }

  /** 计算任务进度百分比 */
  function taskProgress(task) {
    if (!task || !task.total_count) return 0
    const done = (task.completed_count || 0) + (task.failed_count || 0)
    return Math.round((done / task.total_count) * 100)
  }

  /** Continue refreshing pending and running batches without overlapping requests. */
  function startPolling() {
    if (disposed || polling) return
    polling = true
    const epoch = generation
    const tick = async () => {
      if (!polling || disposed || epoch !== generation) return
      if (activeTasks.value.length > 0) await fetchTasks()
      if (polling && !disposed && epoch === generation) {
        pollTimer = setTimeout(tick, POLL_INTERVAL_MS)
      }
    }
    pollTimer = setTimeout(tick, POLL_INTERVAL_MS)
  }

  function stopPolling() {
    polling = false
    generation++
    clearTimeout(pollTimer)
    pollTimer = null
    for (const controller of controllers) controller.abort()
    controllers.clear()
    listRequest = null
    itemRequests.clear()
    if (!disposed) loading.value = false
  }
  onBeforeUnmount(() => {
    stopPolling()
    disposed = true
  })

  return {
    tasks,
    activeTasks,
    completedTasks,
    hasAnyTasks,
    expandedTaskIds,
    itemCache,
    loading,
    error,
    fetchTasks,
    fetchItems,
    toggleExpand,
    deleteTask,
    taskProgress,
    startPolling,
    stopPolling,
  }
}
