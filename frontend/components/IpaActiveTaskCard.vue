<template>
  <div class="queue-item">
    <!-- Icon -->
    <AppArtwork
      :src="task.artworkUrl"
      :alt="task.appName"
      :label="task.appName"
      class="queue-item__icon"
    />

    <!-- Info -->
    <div class="queue-item__info">
      <div class="queue-item__name">
        {{ task.appName }}
      </div>
      <div class="queue-item__meta">
        <span>版本 {{ task.version || '未知' }}</span>
        <span>{{ task.accountEmail || task.account?.email || '未知账号' }}</span>
      </div>

      <!-- Progress -->
      <div
        v-if="task.status !== 'failed' && task.progress !== undefined"
        class="queue-item__progress"
      >
        <ProgressBar
          :percent="task.progress"
          :color="task.stage === 'signing' ? 'var(--color-warning)' : 'var(--color-primary)'"
          size="default"
        />
        <div class="queue-item__progress-info">
          <span>{{ localizeProgressStage(task.stage || '下载中') }} {{ task.progress }}%</span>
          <span v-if="task.fileSize">{{ formatFileSize(task.fileSize) }}</span>
        </div>
      </div>

      <!-- Error -->
      <div
        v-if="task.error"
        class="queue-item__error"
      >
        {{ task.error }}
      </div>
    </div>

    <!-- Actions -->
    <div class="queue-item__actions">
      <button
        class="q-btn q-btn--pause"
        :title="paused ? '继续任务' : '暂停任务'"
        @click="emit('toggle-pause', task.id)"
      >
        <SvgIcon
          v-if="!paused"
          class="h-[14px] w-[14px]"
          :icon="pauseIcon"
        />
        <SvgIcon
          v-else
          class="h-[14px] w-[14px]"
          :icon="playIcon"
        />
      </button>
      <button
        class="q-btn"
        title="取消任务"
        @click="emit('remove', task.id)"
      >
        <SvgIcon
          class="h-[14px] w-[14px]"
          :icon="closeIcon"
        />
      </button>
    </div>
  </div>
</template>

<script setup>
import AppArtwork from './AppArtwork.vue'
import ProgressBar from './ProgressBar.vue'
import SvgIcon from './SvgIcon.vue'
import { localizeProgressStage } from '../composables/useDownload.js'
import pauseIcon from '../assets/icons/pause-fill.svg?raw'
import playIcon from '../assets/icons/play-fill.svg?raw'
import closeIcon from '../assets/icons/close.svg?raw'

defineProps({
  task: { type: Object, required: true },
  paused: { type: Boolean, default: false }
})

const emit = defineEmits(['toggle-pause', 'remove'])

const formatFileSize = (bytes) => {
  if (!bytes) return '未知'
  const units = ['B', 'KB', 'MB', 'GB']
  let value = bytes
  let unitIndex = 0
  while (value >= 1024 && unitIndex < units.length - 1) { value /= 1024; unitIndex += 1 }
  return `${value.toFixed(value >= 100 || unitIndex === 0 ? 0 : 1)} ${units[unitIndex]}`
}
</script>

<style scoped>
/* Queue Item */
.queue-item {
 display: flex;
 align-items: center;
 gap: 12px;
 padding: 12px;
 background: var(--color-surface);
 border: 1px solid var(--color-border);
 border-radius: var(--radius-xl);
 transition: opacity 0.2s ease;
}
.queue-item:active {
 opacity: 0.8;
}
.dark .queue-item {
 background: var(--color-surface);
 border-color: var(--color-surface-muted);
}

/* Icon container — override AppArtwork sizing */
.queue-item__icon {
  width: 44px !important;
  height: 44px !important;
  border-radius: var(--radius-lg) !important;
  flex-shrink: 0;
}

/* Info area */
.queue-item__info {
  flex: 1;
  min-width: 0;
}

.queue-item__name {
 font-size: var(--font-size-body);
 font-weight: 600;
 color: var(--color-text);
 white-space: nowrap;
 overflow: hidden;
 text-overflow: ellipsis;
}
.dark .queue-item__name {
 color: var(--color-text);
}

.queue-item__meta {
 display: flex;
 gap: 10px;
 font-size: var(--font-size-micro);
 color: var(--color-text-muted);
 margin-top: 2px;
}
.dark .queue-item__meta {
 color: var(--color-text-muted);
}

/* Progress area */
.queue-item__progress {
 margin-top: 6px;
}

.queue-item__progress-info {
 display: flex;
 justify-content: space-between;
 font-size: var(--font-size-nano);
 color: var(--color-text-muted);
 margin-top: 3px;
}
.dark .queue-item__progress-info {
 color: var(--color-text-muted);
}

/* Error text */
.queue-item__error {
 font-size: var(--font-size-micro);
 color: var(--color-danger);
 margin-top: 4px;
}

/* Actions */
.queue-item__actions {
  display: flex;
  gap: 4px;
  flex-shrink: 0;
}

/* Action button base */
.q-btn {
 width: 32px;
 height: 32px;
 border-radius: var(--radius-base);
 border: 1px solid var(--color-border);
 background: var(--color-surface);
 color: var(--color-text-muted);
 font-size: var(--font-size-body);
 display: flex;
 align-items: center;
 justify-content: center;
 cursor: pointer;
 transition: all 0.2s ease;
 -webkit-tap-highlight-color: transparent;
 touch-action: manipulation;
 user-select: none;
 -webkit-user-select: none;
 padding: 0;
 position: relative;
}

@media (pointer: coarse) {
  .q-btn::before {
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
.dark .q-btn {
 background: var(--color-surface);
 border-color: var(--color-surface-muted);
 color: var(--color-text-muted);
}
.q-btn:active {
  opacity: 0.7;
}

.q-btn--pause {
  color: var(--color-warning);
  border-color: var(--color-warning-border);
  background: var(--color-warning-bg);
}
.dark .q-btn--pause {
  color: var(--color-warning);
  border-color: rgba(var(--rgb-amber), 0.5);
  background: rgba(var(--rgb-amber), 0.1);
}
</style>
