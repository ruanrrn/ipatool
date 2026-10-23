<template>
  <div class="queue-item queue-item--done">
    <!-- Icon -->
    <AppArtwork
      :src="item.artworkUrl"
      :alt="item.appName"
      :label="item.appName"
      class="queue-item__icon"
    />

    <!-- Info -->
    <div class="queue-item__info">
      <div class="queue-item__name">
        {{ item.appName }}
      </div>
      <div class="queue-item__meta">
        <span>v{{ item.version || '未知' }}</span>
        <span>{{ formatFileSize(item.fileSize) }}</span>
      </div>
    </div>

    <!-- Actions -->
    <div class="queue-item__actions">
      <button
        class="q-btn q-btn--download"
        title="下载"
        @click="emit('download', item.downloadUrl)"
      >
        <SvgIcon
          class="h-[14px] w-[14px]"
          :icon="downloadIcon"
        />
      </button>
      <button
        v-if="item.otaInstallable && item.installUrl"
        class="q-btn q-btn--install"
        title="安装"
        @click="emit('install', item.installUrl)"
      >
        <SvgIcon
          class="h-[14px] w-[14px]"
          :icon="layersIcon"
        />
      </button>
      <button
        class="q-btn q-btn--danger"
        title="删除"
        @click="emit('remove', item)"
      >
        <SvgIcon
          class="h-[14px] w-[14px]"
          :icon="trashIcon"
        />
      </button>
    </div>
  </div>
</template>

<script setup>
import AppArtwork from './AppArtwork.vue'
import SvgIcon from './SvgIcon.vue'
import downloadIcon from '../assets/icons/download.svg?raw'
import layersIcon from '../assets/icons/layers.svg?raw'
import trashIcon from '../assets/icons/trash.svg?raw'

defineProps({
  item: { type: Object, required: true }
})

const emit = defineEmits(['download', 'install', 'remove'])

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
.dark .queue-item--done {
 background: var(--color-surface, #18181b);
 border-color: var(--color-surface-muted, #27272a);
}

/* Queue Item */
.queue-item {
 display: flex;
 align-items: center;
 gap: 12px;
 padding: 12px;
 background: var(--color-surface, #fff);
 border: 1px solid var(--color-border, #ebebeb);
 border-radius: var(--radius-xl);
 transition: opacity 0.2s ease;
}
.queue-item:active {
 opacity: 0.8;
}
.dark .queue-item {
 background: var(--color-surface, #18181b);
 border-color: var(--color-surface-muted, #27272a);
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
 color: var(--color-text, #0d0d0d);
 white-space: nowrap;
 overflow: hidden;
 text-overflow: ellipsis;
}
.dark .queue-item__name {
 color: var(--color-text, #f5f5f5);
}

.queue-item__meta {
 display: flex;
 gap: 10px;
 font-size: var(--font-size-micro);
 color: var(--color-text-muted, #6e6e80);
 margin-top: 2px;
}
.dark .queue-item__meta {
 color: var(--color-text-muted, #a1a1aa);
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
 background: var(--color-surface, #18181b);
 border-color: var(--color-surface-muted, #27272a);
 color: var(--color-text-muted, #a1a1aa);
}
.q-btn:active {
  opacity: 0.7;
}

.q-btn--download {
  color: var(--color-primary);
  border-color: var(--color-primary-border);
}
.dark .q-btn--download {
  color: var(--color-primary);
  border-color: rgba(16, 163, 127, 0.3);
  background: rgba(16, 163, 127, 0.15);
}

.q-btn--install {
  color: var(--color-primary);
  border-color: var(--color-primary-border);
}
.dark .q-btn--install {
  color: var(--color-primary);
  border-color: rgba(16, 163, 127, 0.3);
  background: rgba(16, 163, 127, 0.15);
}

.q-btn--danger {
  color: var(--color-danger);
  border-color: var(--color-danger-border);
}
.dark .q-btn--danger {
  color: var(--color-danger);
  border-color: rgba(239, 68, 68, 0.3);
  background: rgba(239, 68, 68, 0.15);
}
</style>
