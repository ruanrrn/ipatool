<template>
  <Transition name="sheet-fade">
    <div
      v-if="app"
      class="version-sheet-overlay"
      @click.self="emit('close')"
    >
      <Transition name="sheet-slide">
        <div
          v-if="app"
          class="version-sheet"
        >
          <div
            class="version-sheet__handle"
            @click="emit('close')"
          />

          <div class="version-sheet__header">
            <img
              v-if="!app.isDirectAppId && (app.artworkUrl100 || app.artworkUrl60)"
              :src="app.artworkUrl100 || app.artworkUrl60"
              :alt="app.trackName"
              class="version-sheet__icon"
            >
            <div
              v-else
              class="version-sheet__icon version-sheet__icon--placeholder"
            >
              <SvgIcon
                class="w-6 h-6 text-txt-secondary"
                :icon="documentIcon"
              />
            </div>
            <div class="version-sheet__header-info">
              <h3 class="version-sheet__app-name">
                {{ app.trackName }}
              </h3>
              <p class="version-sheet__app-meta">
                {{ app.artistName }}
              </p>
            </div>
            <button
              class="version-sheet__close"
              @click="emit('close')"
            >
              <SvgIcon
                class="w-5 h-5"
                :icon="closeIcon"
              />
            </button>
          </div>

          <div class="version-sheet__body">
            <div class="version-sheet__section version-sheet__section--versions">
              <VersionPicker
                :versions="versions"
                :selected-version="selectedVersion"
                :versions-fetched="versionsFetched"
                :fetching-versions="fetchingVersions"
                :appid="appid"
                :format-file-size="formatFileSize"
                @version-selected="emit('version-selected', $event)"
                @manual-version-request="emit('manual-version-request')"
              />
            </div>

            <div
              v-if="purchaseRequired && versionsFetched"
              class="version-sheet__section"
            >
              <div class="download-disabled-hint">
                ⚠️ {{ downloadBlockedReason }}
              </div>
            </div>

            <div
              v-if="versionsFetched && !purchaseRequired"
              class="version-sheet__section"
            >
              <label class="version-sheet__note-label">备注</label>
              <MobileInput
                :model-value="versionNote"
                placeholder="可选，给这个版本加个备注..."
                clearable
                class="version-sheet__note-input"
                @update:model-value="emit('update:versionNote', $event)"
              />
              <p class="version-sheet__note-hint">
                备注仅在收藏后展示，不填写则为空
              </p>
            </div>

            <ProgressPanel
              :download-url="currentVersionDownloadUrl"
              :install-url="currentVersionInstallUrl"
              :file-size="currentVersionFileSize"
              :ota-installable="currentVersionOtaInstallable"
              :install-method="currentVersionInstallMethod"
              :inspection="currentVersionInspection"
              :is-https="isHttps"
              :current-protocol="currentProtocol"
              @download-ipa="emit('download-ipa')"
              @install-ipa="emit('install-ipa')"
            />
          </div>

          <div
            v-if="versionsFetched && versions.length > 0"
            class="version-sheet__actions"
            :class="{ 'version-sheet__actions--purchase': purchaseRequired }"
          >
            <MobileButton
              v-if="purchaseRequired"
              :disabled="accountMissing || checkingPurchaseStatus"
              :loading="claimRequired && claimingSelectedApp"
              :title="checkingPurchaseStatus ? '正在检测购买状态…' : ''"
              type="primary"
              class="version-sheet__purchase-btn version-sheet__purchase-btn--dock"
              @click="emit('buy-or-claim')"
            >
              <template #icon>
                <i><ArrowRight /></i>
              </template>
              {{ purchaseActionLabel }}
            </MobileButton>
            <template v-else>
              <button
                v-if="showCurrentVersionProgressCard"
                class="version-sheet__action-btn version-sheet__action-btn--progress"
                disabled
                aria-disabled="true"
              >
                <i><component :is="currentVersionProgressMode === 'installing' ? Install : Download" /></i>
                <span>{{ currentVersionProgressButtonLabel }}</span>
              </button>
              <template v-else>
                <button
                  class="version-sheet__action-btn version-sheet__action-btn--secondary"
                  :disabled="accountMissing || downloadBlocked || isCurrentVersionDownloaded"
                  :class="{ 'is-disabled': downloadBlocked || isCurrentVersionDownloaded }"
                  :title="downloadButtonTitle"
                  @click="emit('download')"
                >
                  <template v-if="isCurrentVersionDownloaded">
                    <SvgIcon
                      class="h-4 w-4"
                      :icon="checkIcon"
                    />
                    <span>已下载</span>
                  </template>
                  <template v-else>
                    <i><Download /></i>
                    <span>下载</span>
                  </template>
                </button>
                <button
                  class="version-sheet__action-btn version-sheet__action-btn--primary"
                  :disabled="accountMissing || downloadBlocked"
                  :class="{ 'is-disabled': downloadBlocked }"
                  :title="installButtonTitle"
                  @click="emit('install')"
                >
                  <i><Install /></i>
                  <span>安装</span>
                </button>
              </template>
              <button
                class="version-sheet__action-btn version-sheet__action-btn--fav"
                :class="{ 'is-active': isCurrentAppFavorited }"
                :disabled="favoriteLoading || downloading"
                @click="emit('toggle-favorite')"
              >
                <i><component :is="isCurrentAppFavorited ? StarFilled : Star" /></i>
              </button>
            </template>
          </div>
        </div>
      </Transition>
    </div>
  </Transition>
</template>

<script setup>
import { computed } from 'vue'

import { ArrowRight, Download, Install, Star, StarFilled } from './icons'
import MobileButton from './MobileButton.vue'
import MobileInput from './MobileInput.vue'
import ProgressPanel from './ProgressPanel.vue'
import SvgIcon from './SvgIcon.vue'
import VersionPicker from './VersionPicker.vue'
import checkIcon from '../assets/icons/check.svg?raw'
import closeIcon from '../assets/icons/close.svg?raw'
import documentIcon from '../assets/icons/document.svg?raw'

const props = defineProps({
  app: { type: Object, default: null },
  versions: { type: Array, default: () => [] },
  selectedVersion: { type: String, default: '' },
  versionsFetched: { type: Boolean, default: false },
  fetchingVersions: { type: Boolean, default: false },
  appid: { type: String, default: '' },
  formatFileSize: { type: Function, required: true },
  purchaseRequired: { type: Boolean, default: false },
  downloadBlockedReason: { type: String, default: '' },
  versionNote: { type: String, default: '' },
  currentVersionDownloadUrl: { type: String, default: '' },
  currentVersionInstallUrl: { type: String, default: '' },
  currentVersionFileSize: { type: Number, default: 0 },
  currentVersionOtaInstallable: { type: Boolean, default: false },
  currentVersionInstallMethod: { type: String, default: '' },
  currentVersionInspection: { type: Object, default: null },
  isHttps: { type: Boolean, default: false },
  currentProtocol: { type: String, default: '' },
  selectedAccount: { type: [Number, String, null], default: null },
  checkingPurchaseStatus: { type: Boolean, default: false },
  claimRequired: { type: Boolean, default: false },
  claimingSelectedApp: { type: Boolean, default: false },
  purchaseActionLabel: { type: String, default: '' },
  showCurrentVersionProgressCard: { type: Boolean, default: false },
  currentVersionProgressMode: { type: String, default: 'downloading' },
  currentVersionProgressButtonLabel: { type: String, default: '' },
  downloadBlocked: { type: Boolean, default: false },
  isCurrentVersionDownloaded: { type: Boolean, default: false },
  isCurrentAppFavorited: { type: Boolean, default: false },
  favoriteLoading: { type: Boolean, default: false },
  downloading: { type: Boolean, default: false }
})

const emit = defineEmits([
  'close',
  'version-selected',
  'manual-version-request',
  'update:versionNote',
  'download-ipa',
  'install-ipa',
  'buy-or-claim',
  'download',
  'install',
  'toggle-favorite'
])

const accountMissing = computed(() => !props.selectedAccount && props.selectedAccount !== 0)

const downloadButtonTitle = computed(() => {
  if (props.downloadBlocked) return props.checkingPurchaseStatus ? '正在检测购买状态…' : '请先获取应用'
  return props.isCurrentVersionDownloaded ? '已下载' : '下载 IPA'
})

const installButtonTitle = computed(() => {
  if (props.downloadBlocked) return props.checkingPurchaseStatus ? '正在检测购买状态…' : '请先获取应用'
  return '安装到设备'
})
</script>

<style scoped>
.version-sheet-overlay {
  position: fixed;
  top: 0;
  left: 0;
  right: 0;
  bottom: 0;
  background: var(--overlay-sheet);
  z-index: 1000;
  display: flex;
  align-items: flex-end;
  justify-content: center;
}

.version-sheet {
  position: fixed;
  left: 0;
  right: 0;
  bottom: 0;
  width: min(100%, 720px);
  max-height: min(82svh, calc(100dvh - 12px));
  background: var(--color-surface);
  border-radius: 20px 20px 0 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  box-shadow: var(--shadow-dialog);
}

.version-sheet__handle {
 width: 36px;
 height: 4px;
 background: var(--color-border-divider);
 border-radius: 2px;
 margin: 10px auto 6px;
 cursor: pointer;
 flex-shrink: 0;
}

.version-sheet__header {
 display: flex;
 align-items: center;
 gap: 12px;
 padding: 8px 20px 12px;
 flex-shrink: 0;
}

.version-sheet__icon {
 width: 48px;
 height: 48px;
 border-radius: 12px;
 object-fit: cover;
 flex-shrink: 0;
}

.version-sheet__icon--placeholder {
 background: var(--color-surface-muted);
 border: 1px solid var(--color-border);
 display: flex;
 align-items: center;
 justify-content: center;
}

.version-sheet__header-info {
 flex: 1;
 min-width: 0;
}

.version-sheet__app-name {
 font-size: 17px;
 font-weight: 700;
 color: var(--color-text);
 line-height: 1.3;
 overflow: hidden;
 text-overflow: ellipsis;
 white-space: nowrap;
}

.version-sheet__app-meta {
 font-size: 13px;
 color: var(--color-text-muted);
 line-height: 1.3;
 margin-top: 2px;
 overflow: hidden;
 text-overflow: ellipsis;
 white-space: nowrap;
}

.version-sheet__close {
 width: 32px;
 height: 32px;
 display: flex;
 align-items: center;
 justify-content: center;
 border: none;
 background: var(--color-surface-muted);
 border-radius: 50%;
 cursor: pointer;
 color: var(--color-text-muted);
 flex-shrink: 0;
 transition: background 0.2s;
}

.version-sheet__close:active {
  background: var(--color-border);
}

.version-sheet__body {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  overscroll-behavior: contain;
  -webkit-overflow-scrolling: touch;
  padding-bottom: 8px;
}

.version-sheet__section {
  padding: 0 20px;
  margin-bottom: 12px;
}

.version-sheet__section--versions {
  margin-bottom: 0;
}

.version-sheet__section:has(.version-sheet__note-input) {
  margin-bottom: 0;
}

.version-sheet__purchase-btn {
 width: 100%;
 margin-top: 8px;
 border-radius: 10px;
 height: 44px;
}

.version-sheet__purchase-btn--dock {
 margin-top: 0;
 height: 48px;
}

.version-sheet__note-label {
  font-size: 13px;
  font-weight: 600;
  color: var(--color-text);
  display: block;
  margin-bottom: 6px;
}

.version-sheet__note-input {
  width: 100%;
}

.version-sheet__note-hint {
  font-size: 11px;
  color: var(--color-text-tertiary);
  margin-top: 4px;
}

.version-sheet__actions {
  position: sticky;
  bottom: 0;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 12px 20px;
  padding-bottom: calc(12px + env(safe-area-inset-bottom, 0px));
  border-top: 1px solid var(--color-border);
  background: var(--color-surface);
  box-shadow: 0 -8px 24px rgba(var(--rgb-slate-900), 0.06);
  flex-shrink: 0;
  z-index: 2;
  min-height: 48px;
  transition: all 0.2s ease;
}

.version-sheet__actions--purchase {
 display: block;
}

.version-sheet__action-btn {
 display: flex;
 align-items: center;
 justify-content: center;
 gap: 6px;
 border: none;
 border-radius: 10px;
 height: 44px;
 font-size: 14px;
 font-weight: 600;
 cursor: pointer;
 transition: all 0.15s ease;
 flex: 1;
 min-width: 0;
}

.version-sheet__action-btn i,
.version-sheet__action-btn svg {
 width: 18px;
 height: 18px;
 flex-shrink: 0;
}

.version-sheet__action-btn--secondary {
  background: var(--color-surface-muted);
  color: var(--color-text);
  border: 1px solid var(--color-border);
}

.version-sheet__action-btn--secondary:active {
  background: var(--color-surface-hover);
}

.version-sheet__action-btn--secondary.is-disabled {
 opacity: 0.5;
 cursor: not-allowed;
}

.version-sheet__action-btn--primary {
  background: var(--color-primary);
  color: var(--color-text-inverse);
  flex: 1.3;
}

.version-sheet__action-btn--primary:active {
  background: var(--color-primary-hover);
}

.version-sheet__action-btn--primary.is-disabled {
 opacity: 0.5;
 cursor: not-allowed;
}

.version-sheet__action-btn--progress {
  background: var(--color-primary);
  color: var(--color-text-inverse);
  flex: 1;
  cursor: not-allowed;
  opacity: 0.92;
}

.version-sheet__action-btn--progress[disabled] {
  pointer-events: none;
}

.version-sheet__action-btn--fav {
  background: var(--color-warning-soft);
  color: var(--color-warning-dark);
  width: 48px;
  flex: none;
}

.version-sheet__action-btn--fav:active {
  background: var(--color-warning-border);
}

.version-sheet__action-btn--fav.is-active {
  background: var(--color-warning);
  color: var(--color-text-inverse);
}

.sheet-fade-enter-active {
 transition: opacity 0.25s ease;
}

.sheet-fade-leave-active {
 transition: opacity 0.2s ease;
}

.sheet-fade-enter-from,
.sheet-fade-leave-to {
 opacity: 0;
}

.sheet-slide-enter-active {
 transition: transform 0.3s cubic-bezier(0.32, 0.72, 0, 1);
}

.sheet-slide-leave-active {
 transition: transform 0.2s cubic-bezier(0.32, 0.72, 0, 1);
}

.sheet-slide-enter-from,
.sheet-slide-leave-to {
 transform: translateY(100%);
}

.dark .version-sheet {
  background: var(--color-surface);
  box-shadow: var(--shadow-dialog);
}

.dark .version-sheet__handle {
  background: var(--color-border-handle);
}

.dark .version-sheet__app-name {
 color: var(--color-text);
}

.dark .version-sheet__app-meta {
 color: var(--color-text-muted);
}

.dark .version-sheet__close {
 background: var(--color-surface-muted);
 color: var(--color-text-muted);
}

.dark .version-sheet__close:active {
  background: var(--color-border);
}

.dark .version-sheet__note-label {
 color: var(--color-text);
}

.dark .version-sheet__actions {
  border-top-color: var(--color-surface-muted);
  background: var(--color-surface);
}

.dark .version-sheet__action-btn--secondary {
  background: var(--color-surface-muted);
  color: var(--color-text);
  border-color: var(--color-border);
}

.dark .version-sheet__action-btn--fav {
  background: var(--color-warning-soft);
  color: var(--color-warning);
}

.dark .version-sheet__action-btn--fav.is-active {
  background: var(--color-warning);
  color: var(--color-text-inverse);
}

.dark .download-disabled-hint {
  background: var(--color-danger-bg-dark);
  border-color: var(--color-surface-muted);
  color: var(--color-text-muted);
}
</style>
