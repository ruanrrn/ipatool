import { onBeforeUnmount } from 'vue'
import { API_BASE } from '../config.js'
import { apiFetch } from '../utils/api.js'

/** Track each task from its first request through completion or interruption. */
export function useJobPolling(options = {}) {
  const {
    isFinalStatus = status => ['completed', 'ready', 'failed', 'error'].includes(status),
    pollInterval = 1500, maxFailures = 5,
    onComplete, onFailed, onUpdate, onError, onInterrupted
  } = options
  const runs = new Map()
  const failures = new Map()
  let disposed = false
  const stopPolling = taskId => {
    const run = runs.get(taskId)
    runs.delete(taskId)
    if (run) {
      clearTimeout(run.timer)
      run.controller.abort()
    }
    failures.delete(taskId)
  }
  const stopAllPolling = () => {
    for (const id of runs.keys()) stopPolling(id)
  }
  const pollOnce = async (taskId, run) => {
    const current = () => !disposed && (!run || runs.get(taskId) === run)
    const interrupt = reason => {
      stopPolling(taskId)
      onInterrupted?.(taskId, reason)
    }
    try {
      const { response, data } = await apiFetch(
        `${API_BASE}/job-info?jobId=${encodeURIComponent(taskId)}`,
        run ? { signal: run.controller.signal } : {}, { retries: 0 }
      )
      if (!current()) return null
      if (response.status === 404) {
        interrupt('任务已不存在')
        return null
      }
      if (!response.ok || !data?.ok || !data.data) throw new Error(data?.error || '任务状态获取失败')
      failures.delete(taskId)
      const snapshot = data.data
      onUpdate?.(taskId, snapshot)
      // Dispatch terminal events before a Vue watcher can remove the run after onUpdate.
      if (run && current() && isFinalStatus(snapshot.status)) {
        stopPolling(taskId)
        if (['ready', 'completed'].includes(snapshot.status)) onComplete?.(taskId, snapshot)
        else onFailed?.(taskId, snapshot)
      }
      return snapshot
    } catch (error) {
      if (!current()) return null
      const count = (failures.get(taskId) || 0) + 1
      failures.set(taskId, count)
      onError?.(taskId, error.message, count)
      if (count >= maxFailures) interrupt('轮询多次失败，请检查网络')
      return null
    }
  }
  const startPolling = (taskId, taskOptions = {}) => {
    if (disposed || runs.has(taskId)) return false
    const run = { controller: new AbortController(), timer: null }
    runs.set(taskId, run)
    const tick = async () => {
      await pollOnce(taskId, run)
      if (runs.get(taskId) !== run || disposed) return
      run.timer = setTimeout(tick, taskOptions.pollInterval ?? pollInterval)
    }
    void tick()
    return true
  }
  const ensurePolling = task => {
    if (!task?.id || isFinalStatus(task.status)) return false
    return runs.has(task.id) || startPolling(task.id)
  }
  const syncPolling = (tasks, taskKey = 'id') => {
    const activeIds = new Set()
    for (const task of tasks) {
      const id = task?.[taskKey]
      if (id && !isFinalStatus(task.status)) {
        activeIds.add(id)
        ensurePolling({ ...task, id })
      }
    }
    for (const id of runs.keys()) if (!activeIds.has(id)) stopPolling(id)
  }
  onBeforeUnmount(() => {
    disposed = true
    stopAllPolling()
  })
  return {
    startPolling, stopPolling, stopAllPolling, resetPolling: stopPolling,
    ensurePolling, syncPolling, pollOnce,
    getPollingStatus: id => ({ isActive: runs.has(id), failureCount: failures.get(id) || 0 })
  }
}
