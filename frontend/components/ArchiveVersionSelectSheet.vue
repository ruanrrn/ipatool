<template>
  <MobileDialog
    :model-value="modelValue"
    :title="title"
    position="bottom"
    @update:model-value="emit('update:modelValue', $event)"
  >
    <div class="archive-version-sheet">
      <div class="archive-version-sheet__app">
        <AppArtwork
          :src="app?.icon_url"
          :alt="app?.name"
          :label="app?.name"
          class="archive-version-sheet__icon"
        />
        <div class="archive-version-sheet__app-info">
          <div class="archive-version-sheet__name">
            {{ app?.name || '未知应用' }}
          </div>
          <div class="archive-version-sheet__meta">
            {{ app?.bundle_id || app?.artist_name || '选择要下载的归档版本' }}
          </div>
        </div>
      </div>

      <div
        v-if="loading"
        class="archive-version-sheet__loading"
      >
        正在加载版本…
      </div>
      <div
        v-else-if="!versions.length"
        class="archive-version-sheet__empty"
      >
        未获取到可用版本
      </div>
      <div
        v-else
        class="archive-version-sheet__list"
      >
        <button
          v-for="version in versions"
          :key="version.version_id || version.version"
          type="button"
          class="archive-version-option"
          :class="{ 'archive-version-option--selected': isSelected(version) }"
          @click="emit('select', getVersionId(version))"
        >
          <span class="archive-version-option__radio">
            <span
              v-if="isSelected(version)"
              class="archive-version-option__radio-fill"
            />
          </span>
          <span class="archive-version-option__content">
            <span class="archive-version-option__top">
              <span class="archive-version-option__ver">v{{ version.version || version.version_id }}</span>
              <span
                v-if="version.description"
                class="archive-version-option__desc"
              >{{ version.description }}</span>
            </span>
            <span class="archive-version-option__id">ID: {{ version.version_id }}</span>
          </span>
        </button>
      </div>
    </div>

    <template #footer>
      <div class="archive-version-sheet__footer">
        <MobileButton
          size="small"
          @click="emit('update:modelValue', false)"
        >
          取消
        </MobileButton>
        <MobileButton
          type="primary"
          size="small"
          :disabled="!selectedVersionId || loading"
          @click="emit('download')"
        >
          下载所选版本
        </MobileButton>
      </div>
    </template>
  </MobileDialog>
</template>

<script setup>
import MobileDialog from './MobileDialog.vue'
import MobileButton from './MobileButton.vue'
import AppArtwork from './AppArtwork.vue'
import { getVersionId } from '../utils/version.js'

const props = defineProps({
  modelValue: {
    type: Boolean,
    default: false
  },
  app: {
    type: Object,
    default: null
  },
  versions: {
    type: Array,
    default: () => []
  },
  selectedVersionId: {
    type: String,
    default: ''
  },
  loading: {
    type: Boolean,
    default: false
  },
  title: {
    type: String,
    default: '选择下载版本'
  }
})

const emit = defineEmits(['update:modelValue', 'select', 'download'])

const isSelected = (version) => String(props.selectedVersionId || '') === getVersionId(version)
</script>

<style scoped>
.archive-version-sheet {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.archive-version-sheet__app {
  display: flex;
  align-items: center;
  gap: 12px;
}

.archive-version-sheet__icon {
  width: 44px;
  height: 44px;
  border-radius: 12px;
  flex-shrink: 0;
}

.archive-version-sheet__app-info {
  min-width: 0;
  flex: 1;
}

.archive-version-sheet__name {
  font-size: var(--font-size-body);
  font-weight: 700;
  color: var(--color-txt);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.archive-version-sheet__meta,
.archive-version-option__id,
.archive-version-option__desc,
.archive-version-sheet__loading,
.archive-version-sheet__empty {
  font-size: var(--font-size-caption);
  color: var(--color-txt-secondary);
}

.archive-version-sheet__list {
  display: flex;
  flex-direction: column;
  gap: 6px;
  max-height: min(360px, 48vh);
  overflow-y: auto;
  overscroll-behavior: contain;
  -webkit-overflow-scrolling: touch;
  padding-right: 4px;
  content-visibility: auto;
  contain-intrinsic-size: auto 56px;
}

.archive-version-option {
  display: flex;
  align-items: center;
  gap: 12px;
  width: 100%;
  padding: 12px;
  border: 1px solid transparent;
  border-radius: var(--radius-xl, 12px);
  background: var(--color-bg-secondary);
  text-align: left;
  cursor: pointer;
  transition: all 0.2s ease;
  touch-action: manipulation;
  -webkit-tap-highlight-color: transparent;
  user-select: none;
  -webkit-user-select: none;
}

.archive-version-option--selected {
  background: var(--color-bg-primary);
  border-color: var(--color-primary);
  box-shadow: 0 2px 8px rgba(var(--color-primary-rgb), 0.1);
}

.archive-version-option__radio {
  position: relative;
  width: 20px;
  height: 20px;
  border: 2px solid var(--color-border);
  border-radius: 50%;
  flex-shrink: 0;
}

.archive-version-option--selected .archive-version-option__radio {
  border-color: var(--color-primary);
}

.archive-version-option__radio-fill {
  position: absolute;
  top: 50%;
  left: 50%;
  width: 10px;
  height: 10px;
  transform: translate(-50%, -50%);
  border-radius: 50%;
  background: var(--color-primary);
}

.archive-version-option__content,
.archive-version-option__top {
  display: flex;
  flex-direction: column;
  gap: 3px;
  min-width: 0;
}

.archive-version-option__ver {
  font-size: var(--font-size-body);
  font-weight: 700;
  color: var(--color-txt);
}

.archive-version-sheet__footer {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}
</style>
