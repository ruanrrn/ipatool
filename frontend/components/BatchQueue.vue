<template>
  <div class="batch-queue">
    <!-- Header -->
    <div class="batch-queue__header">
      <div class="batch-queue__title-wrap">
        <h2 class="batch-queue__title">
          批量下载
        </h2>
        <span
          v-if="hasAnyTasks"
          class="batch-queue__count"
        >
          {{ tasks.length }} 个任务
        </span>
      </div>
      <button
        class="batch-queue__refresh"
        type="button"
        :disabled="loading"
        @click="fetchTasks"
      >
        刷新
      </button>
    </div>

    <!-- Loading -->
    <div
      v-if="loading && tasks.length === 0"
      class="batch-queue__empty"
    >
      <div class="skeleton skeleton--card" />
      <div class="skeleton skeleton--card" />
    </div>

    <!-- Empty -->
    <div
      v-else-if="!hasAnyTasks"
      class="batch-queue__empty-state"
    >
      <div class="batch-queue__empty-icon">
        📦
      </div>
      <p class="batch-queue__empty-text">
        暂无批量下载任务
      </p>
      <p class="batch-queue__empty-desc">
        通过 API 创建批量任务后将在此显示
      </p>
    </div>

    <!-- Task List -->
    <div
      v-else
      class="batch-queue__list"
    >
      <div
        v-for="task in tasks"
        :key="task.id"
        class="batch-task"
        :class="{ 'batch-task--expanded': expandedTaskIds.has(task.id) }"
      >
        <!-- Task Header (clickable) -->
        <button
          class="batch-task__header"
          type="button"
          @click="toggleExpand(task)"
        >
          <div class="batch-task__info">
            <div class="batch-task__name-row">
              <span
                class="batch-task__status-dot"
                :class="`batch-task__status-dot--${task.status}`"
              />
              <span class="batch-task__name">{{ task.task_name }}</span>
            </div>
            <div class="batch-task__meta">
              <span class="batch-task__meta-item">
                {{ task.completed_count || 0 }} / {{ task.total_count }} 完成
              </span>
              <span
                v-if="task.failed_count > 0"
                class="batch-task__meta-item batch-task__meta-item--failed"
              >
                {{ task.failed_count }} 失败
              </span>
              <span class="batch-task__meta-item batch-task__meta-item--time">
                {{ formatTime(task.created_at) }}
              </span>
            </div>
            <!-- Progress bar -->
            <div class="batch-task__progress-track">
              <div
                class="batch-task__progress-fill"
                :class="`batch-task__progress-fill--${task.status}`"
                :style="{ width: `${taskProgress(task)}%` }"
              />
            </div>
          </div>
          <div class="batch-task__actions">
            <span class="batch-task__percent">{{ taskProgress(task) }}%</span>
            <SvgIcon
              class="batch-task__chevron"
              :class="{ 'batch-task__chevron--open': expandedTaskIds.has(task.id) }"
              :icon="chevronDownIcon"
            />
          </div>
        </button>

        <!-- Expanded Items -->
        <div
          v-if="expandedTaskIds.has(task.id)"
          class="batch-task__items"
        >
          <div
            v-if="!itemCache[task.id] || itemCache[task.id].length === 0"
            class="batch-task__items-empty"
          >
            加载中...
          </div>
          <div
            v-for="item in (itemCache[task.id] || [])"
            :key="item.id"
            class="batch-item"
          >
            <div class="batch-item__icon">
              <span
                class="batch-item__status"
                :class="`batch-item__status--${item.status}`"
              >{{ itemStatusIcon(item.status) }}</span>
            </div>
            <div class="batch-item__body">
              <div class="batch-item__name">
                {{ item.app_name || `App ${item.app_id}` }}
              </div>
              <div class="batch-item__sub">
                <span v-if="item.version">v{{ item.version }}</span>
                <span class="batch-item__email">{{ maskEmail(item.account_email) }}</span>
              </div>
              <div
                v-if="item.error"
                class="batch-item__error"
              >
                {{ item.error }}
              </div>
            </div>
            <div class="batch-item__right">
              <span
                v-if="item.status === 'pending'"
                class="batch-item__badge batch-item__badge--pending"
              >等待</span>
              <span
                v-else-if="item.status === 'completed'"
                class="batch-item__badge batch-item__badge--completed"
              >完成</span>
              <span
                v-else-if="item.status === 'failed'"
                class="batch-item__badge batch-item__badge--failed"
              >失败</span>
              <span
                v-if="item.retry_count > 0"
                class="batch-item__retry"
              >重试 {{ item.retry_count }}</span>
            </div>
          </div>

          <!-- Delete button -->
          <button
            class="batch-task__delete"
            type="button"
            @click.stop="handleDelete(task)"
          >
            删除任务记录
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { onMounted, onUnmounted } from 'vue'
import SvgIcon from './SvgIcon.vue'
import chevronDownIcon from '../assets/icons/chevron-down.svg?raw'
import { useBatchTasks } from '../composables/useBatchTasks.js'
import { Confirm } from './MobileConfirm.vue'
import { Toast } from './MobileToast.vue'

const {
  tasks,
  hasAnyTasks,
  expandedTaskIds,
  itemCache,
  loading,
  fetchTasks,
  toggleExpand,
  deleteTask,
  taskProgress,
  startPolling,
  stopPolling,
} = useBatchTasks()

onMounted(async () => {
  await fetchTasks()
  startPolling()
})

onUnmounted(() => {
  stopPolling()
})

function formatTime(ts) {
  if (!ts) return ''
  try {
    const d = new Date(ts.replace(' ', 'T') + 'Z')
    const now = new Date()
    const diff = (now - d) / 1000
    if (diff < 60) return '刚刚'
    if (diff < 3600) return `${Math.floor(diff / 60)}分钟前`
    if (diff < 86400) return `${Math.floor(diff / 3600)}小时前`
    return d.toLocaleDateString('zh-CN')
  } catch {
    return ts
  }
}

function maskEmail(email) {
  if (!email) return ''
  const [name, domain] = email.split('@')
  if (!domain) return email
  if (name.length <= 2) return email
  return `${name[0]}***@${domain}`
}

function itemStatusIcon(status) {
  switch (status) {
    case 'completed': return '✓'
    case 'failed': return '✕'
    case 'pending':
    default: return '○'
  }
}

async function handleDelete(task) {
  const ok = await Confirm(`确定删除任务「${task.task_name}」的记录？`, {
    title: '删除批量任务',
    confirmText: '删除',
    danger: true,
  })
  if (!ok) return
  const success = await deleteTask(task.id)
  if (success) {
    Toast({ message: '任务已删除', type: 'success' })
  } else {
    Toast({ message: '删除失败', type: 'error' })
  }
}
</script>

<style scoped>
.batch-queue {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

/* Header */
.batch-queue__header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.batch-queue__title-wrap {
  display: flex;
  align-items: baseline;
  gap: 8px;
}
.batch-queue__title {
  font-size: 18px;
  font-weight: 700;
  color: var(--color-text);
  margin: 0;
}
.dark .batch-queue__title {
  color: var(--color-text);
}
.batch-queue__count {
  font-size: 12px;
  color: var(--color-text-muted);
}
.dark .batch-queue__count {
  color: var(--color-text-muted);
}
.batch-queue__refresh {
  font-size: 13px;
  color: var(--color-primary);
  background: none;
  border: none;
  cursor: pointer;
  padding: 4px 8px;
  border-radius: 6px;
}
.batch-queue__refresh:disabled {
  opacity: 0.5;
  cursor: default;
}

/* Empty */
.batch-queue__empty-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  padding: 48px 24px;
  text-align: center;
}
.batch-queue__empty-icon {
  font-size: 48px;
  margin-bottom: 12px;
  opacity: 0.5;
}
.batch-queue__empty-text {
  font-size: 14px;
  font-weight: 600;
  color: var(--color-text-muted);
  margin: 0 0 4px;
}
.dark .batch-queue__empty-text {
  color: var(--color-text-muted);
}
.batch-queue__empty-desc {
  font-size: 12px;
  color: var(--color-text-muted);
  margin: 0;
}
.dark .batch-queue__empty-desc {
  color: var(--color-text-muted);
}

/* Skeleton */
.skeleton--card {
  height: 72px;
  border-radius: 14px;
  background: var(--color-surface-muted);
  animation: skeleton-pulse 1.5s ease-in-out infinite;
}
@keyframes skeleton-pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.5; }
}

/* Task card */
.batch-task {
  border-radius: 14px;
  background: var(--color-surface);
  border: 1px solid var(--color-border);
  overflow: hidden;
  transition: border-color 0.15s ease;
}
.dark .batch-task {
  background: var(--color-surface);
  border-color: var(--color-border);
}
.batch-task--expanded {
  border-color: var(--color-primary);
}

.batch-task__header {
  display: flex;
  align-items: flex-start;
  gap: 12px;
  width: 100%;
  padding: 14px;
  background: none;
  border: none;
  cursor: pointer;
  text-align: left;
}
.batch-task__info {
  flex: 1;
  min-width: 0;
}
.batch-task__name-row {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 4px;
}
.batch-task__status-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  flex-shrink: 0;
}
.batch-task__status-dot--processing {
  background: var(--color-primary);
  animation: pulse-dot 1.5s ease-in-out infinite;
}
.batch-task__status-dot--completed {
  background: var(--color-text-muted);
}
@keyframes pulse-dot {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.4; }
}
.batch-task__name {
  font-size: 14px;
  font-weight: 600;
  color: var(--color-text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.dark .batch-task__name {
  color: var(--color-text);
}
.batch-task__meta {
  display: flex;
  gap: 12px;
  flex-wrap: wrap;
  font-size: 12px;
  color: var(--color-text-muted);
  margin-bottom: 8px;
}
.dark .batch-task__meta {
  color: var(--color-text-muted);
}
.batch-task__meta-item--failed {
  color: var(--color-danger);
}
.batch-task__meta-item--time {
  opacity: 0.7;
}

/* Progress bar */
.batch-task__progress-track {
  height: 4px;
  border-radius: 2px;
  background: var(--color-border);
  overflow: hidden;
}
.dark .batch-task__progress-track {
  background: var(--color-border);
}
.batch-task__progress-fill {
  height: 100%;
  border-radius: 2px;
  transition: width 0.3s ease;
}
.batch-task__progress-fill--processing {
  background: var(--color-primary);
}
.batch-task__progress-fill--completed {
  background: var(--color-text-muted);
}

.batch-task__actions {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
}
.batch-task__percent {
  font-size: 13px;
  font-weight: 600;
  color: var(--color-text-muted);
  min-width: 36px;
  text-align: right;
}
.dark .batch-task__percent {
  color: var(--color-text-muted);
}
.batch-task__chevron {
  width: 16px;
  height: 16px;
  color: var(--color-text-muted);
  transition: transform 0.2s ease;
}
.batch-task__chevron--open {
  transform: rotate(180deg);
}
.dark .batch-task__chevron {
  color: var(--color-text-muted);
}

/* Expanded items */
.batch-task__items {
  border-top: 1px solid var(--color-border);
  padding: 8px 14px 14px;
}
.dark .batch-task__items {
  border-color: var(--color-border);
}
.batch-task__items-empty {
  padding: 16px;
  text-align: center;
  font-size: 13px;
  color: var(--color-text-muted);
}
.dark .batch-task__items-empty {
  color: var(--color-text-muted);
}

/* Item row */
.batch-item {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  padding: 10px 0;
  border-bottom: 1px solid var(--color-border);
}
.dark .batch-item {
  border-color: var(--color-border);
}
.batch-item:last-of-type {
  border-bottom: none;
}
.batch-item__icon {
  flex-shrink: 0;
  width: 24px;
  text-align: center;
}
.batch-item__status {
  font-size: 14px;
}
.batch-item__status--completed {
  color: var(--color-primary);
}
.batch-item__status--failed {
  color: var(--color-danger);
}
.batch-item__status--pending {
  color: var(--color-text-muted);
}
.dark .batch-item__status--pending {
  color: var(--color-text-muted);
}
.batch-item__body {
  flex: 1;
  min-width: 0;
}
.batch-item__name {
  font-size: 13px;
  font-weight: 500;
  color: var(--color-text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.dark .batch-item__name {
  color: var(--color-text);
}
.batch-item__sub {
  display: flex;
  gap: 8px;
  font-size: 11px;
  color: var(--color-text-muted);
  margin-top: 2px;
}
.dark .batch-item__sub {
  color: var(--color-text-muted);
}
.batch-item__error {
  font-size: 11px;
  color: var(--color-danger);
  margin-top: 4px;
  word-break: break-word;
}
.batch-item__right {
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 2px;
}
.batch-item__badge {
  font-size: 11px;
  padding: 2px 8px;
  border-radius: 10px;
  font-weight: 500;
}
.batch-item__badge--pending {
  background: var(--color-surface-muted);
  color: var(--color-text-muted);
}
.batch-item__badge--completed {
  background: rgba(var(--rgb-brand), 0.12);
  color: var(--color-primary);
}
.batch-item__badge--failed {
  background: rgba(var(--rgb-red), 0.12);
  color: var(--color-danger);
}
.batch-item__retry {
  font-size: 10px;
  color: var(--color-text-muted);
}
.dark .batch-item__retry {
  color: var(--color-text-muted);
}

/* Delete button */
.batch-task__delete {
  width: 100%;
  margin-top: 8px;
  padding: 8px;
  font-size: 12px;
  color: var(--color-danger);
  background: none;
  border: 1px solid var(--color-border);
  border-radius: 8px;
  cursor: pointer;
  transition: background 0.15s ease;
}
.dark .batch-task__delete {
  border-color: var(--color-border);
}
.batch-task__delete:hover {
  background: rgba(var(--rgb-red), 0.06);
}
</style>
