<template>
  <!-- 通用弹窗：支持 center / bottom 两种模式 — Orbit v3 -->
  <Teleport to="body">
    <Transition :name="transitionName">
      <div
        v-if="modelValue"
        class="mobile-dialog"
        @click="onOverlayClick"
      >
        <div
          class="mobile-dialog__panel"
          :class="[`mobile-dialog__panel--${position}`]"
          role="dialog"
          aria-modal="true"
          @click.stop
        >
          <div
            v-if="$slots.icon"
            class="mobile-dialog__icon"
          >
            <slot name="icon" />
          </div>

          <header
            v-if="title"
            class="mobile-dialog__header"
          >
            <div class="mobile-dialog__title">
              {{ title }}
            </div>
            <button
              v-if="showClose"
              class="mobile-dialog__close"
              type="button"
              aria-label="关闭"
              @click="close"
            >
              ✕
            </button>
          </header>

          <div class="mobile-dialog__body">
            <slot />
          </div>

          <footer
            v-if="$slots.footer"
            class="mobile-dialog__footer"
          >
            <slot name="footer" />
          </footer>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<script setup>
import { computed, onBeforeUnmount, watch } from 'vue'

const props = defineProps({
  modelValue: {
    type: Boolean,
    default: false
  },
  title: {
    type: String,
    default: ''
  },
  position: {
    type: String,
    default: 'center',
    validator: (v) => ['center', 'bottom'].includes(v)
  },
  closeOnClickOverlay: {
    type: Boolean,
    default: true
  },
  showClose: {
    type: Boolean,
    default: true
  },
  closeOnEsc: {
    type: Boolean,
    default: true
  }
})

const emit = defineEmits(['update:modelValue'])

const transitionName = computed(() => {
  return props.position === 'bottom' ? 'mobile-dialog-bottom' : 'mobile-dialog-center'
})

const close = () => emit('update:modelValue', false)

const onOverlayClick = () => {
  if (props.closeOnClickOverlay) close()
}

// 轻量的滚动锁定（仅在打开时）
let prevOverflow = ''
const lockScroll = () => {
  prevOverflow = document.documentElement.style.overflow
  document.documentElement.style.overflow = 'hidden'
}
const unlockScroll = () => {
  document.documentElement.style.overflow = prevOverflow || ''
}

watch(
  () => props.modelValue,
  (v) => {
    if (typeof document === 'undefined') return
    if (v) lockScroll()
    else unlockScroll()
  },
  { immediate: true }
)

// Esc 关闭
const onKeydown = (e) => {
  if (!props.modelValue) return
  if (!props.closeOnEsc) return
  if (e.key === 'Escape') {
    e.preventDefault()
    close()
  }
}

watch(
  () => props.modelValue,
  (open) => {
    if (typeof document === 'undefined') return
    if (open) document.addEventListener('keydown', onKeydown)
    else document.removeEventListener('keydown', onKeydown)
  },
  { immediate: true }
)

onBeforeUnmount(() => {
  if (typeof document === 'undefined') return
  unlockScroll()
  document.removeEventListener('keydown', onKeydown)
})
</script>

<style scoped>
.mobile-dialog {
  position: fixed;
  inset: 0;
  z-index: 999;
  display: flex;
  align-items: center;
  justify-content: center;

  background: var(--color-overlay-dialog, rgba(0, 0, 0, 0.5));
  -webkit-tap-highlight-color: transparent;
}

.mobile-dialog__panel {
  width: 100%;
  max-width: 320px;
  border: none;
  box-shadow: var(--shadow-dialog, 0 20px 40px rgba(0, 0, 0, 0.15));

  background: var(--color-surface, #fff);
  overflow: hidden;
}

.mobile-dialog__panel--center {
  margin: 40px;
  border-radius: var(--radius-3xl, 18px);
}

.mobile-dialog__panel--bottom {
  align-self: flex-end;
  border-radius: var(--radius-sheet, 20px) var(--radius-sheet, 20px) 0 0;
  max-width: 100vw;
}

.mobile-dialog__icon {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 48px;
  height: 48px;
  border-radius: 50%;
  margin: 0 auto 12px;
}

.mobile-dialog__header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;

  padding: 24px 24px 0;
}

.mobile-dialog__title {
  font-family: var(--font-body);
  font-size: 17px;
  font-weight: 600;
  color: var(--color-text-primary, #0d0d0d);
  line-height: 1.3;
}

.mobile-dialog__close {
  min-width: 32px;
  min-height: 32px;
  border: none;
  border-radius: var(--radius-lg, 10px);
  background: transparent;
  color: var(--color-text-tertiary, #c0c0c0);
  cursor: pointer;
  font-size: 16px;
  display: flex;
  align-items: center;
  justify-content: center;
  -webkit-tap-highlight-color: transparent;
  touch-action: manipulation;
  user-select: none;
  -webkit-user-select: none;
  position: relative;
  transition: color 0.2s ease;
}

@media (pointer: coarse) {
  .mobile-dialog__close::before {
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

.mobile-dialog__close:active {
  color: var(--color-text-primary, #0d0d0d);
}

.mobile-dialog__body {
  padding: 24px;
  color: var(--color-text-secondary, #6e6e80);
  font-size: 14px;
  line-height: 1.5;

  max-height: 70vh;
  overflow: auto;
  -webkit-overflow-scrolling: touch;
}

.mobile-dialog__panel--bottom .mobile-dialog__body {
  padding-bottom: calc(24px + max(env(safe-area-inset-bottom), var(--kb-inset-bottom, 0px)));
}

.mobile-dialog__footer {
  padding: 0 24px 24px;
}

.mobile-dialog__panel--bottom .mobile-dialog__footer {
  padding-bottom: calc(24px + max(env(safe-area-inset-bottom), var(--kb-inset-bottom, 0px)));
}

.mobile-dialog__panel--bottom:has(.mobile-dialog__footer) .mobile-dialog__body {
  padding-bottom: 16px;
}

/* ==================== Animations ==================== */

/* Center dialog: scale(0.95) + fade, 200ms ease-out enter, 150ms ease-out leave */
.mobile-dialog-center-enter-active {
  transition: opacity 250ms ease-out, transform 200ms ease-out;
}

.mobile-dialog-center-leave-active {
  transition: opacity 200ms ease-in, transform 150ms ease-in;
}

.mobile-dialog-center-enter-from,
.mobile-dialog-center-leave-to {
  opacity: 0;
}

.mobile-dialog-center-enter-from .mobile-dialog__panel--center {
  transform: scale(0.95);
}

.mobile-dialog-center-leave-to .mobile-dialog__panel--center {
  transform: scale(0.95);
}

.mobile-dialog-center-enter-to,
.mobile-dialog-center-leave-from {
  opacity: 1;
  transform: scale(1);
}

/* Bottom sheet: slide up + fade, 300ms cubic-bezier enter, 250ms cubic-bezier leave */
.mobile-dialog-bottom-enter-active {
  transition: opacity 250ms ease-out;
}

.mobile-dialog-bottom-leave-active {
  transition: opacity 200ms ease-in;
}

.mobile-dialog-bottom-enter-from,
.mobile-dialog-bottom-leave-to {
  opacity: 0;
}

.mobile-dialog-bottom-enter-from .mobile-dialog__panel--bottom {
  transform: translateY(100%);
}

.mobile-dialog-bottom-leave-to .mobile-dialog__panel--bottom {
  transform: translateY(100%);
}

.mobile-dialog-bottom-enter-to .mobile-dialog__panel--bottom,
.mobile-dialog-bottom-leave-from .mobile-dialog__panel--bottom {
  transform: translateY(0);
}

/* Bottom panel slide animation with iOS curve */
.mobile-dialog-bottom-enter-active .mobile-dialog__panel--bottom {
  transition: transform 300ms cubic-bezier(0.32, 0.72, 0, 1);
}

.mobile-dialog-bottom-leave-active .mobile-dialog__panel--bottom {
  transition: transform 250ms cubic-bezier(0.32, 0.72, 0, 1);
}

/* 深色模式 */
.dark .mobile-dialog {
  background: rgba(0, 0, 0, 0.7);
}

.dark .mobile-dialog__panel {
  background: var(--color-surface, #18181b);
  box-shadow: 0 20px 40px rgba(0, 0, 0, 0.5);
}

.dark .mobile-dialog__title {
  color: var(--color-text, #f5f5f5);
}

.dark .mobile-dialog__body {
  color: var(--color-text-muted, #a1a1aa);
}

.dark .mobile-dialog__close {
  color: var(--color-text-tertiary, #71717a);
}

.dark .mobile-dialog__close:active {
  color: var(--color-text, #f5f5f5);
}

/* 高对比度 */
@media (prefers-contrast: high) {
  .mobile-dialog__panel {
    border: 2px solid var(--color-border-default, #ebebeb);
  }
}

/* 减少动画 */
@media (prefers-reduced-motion: reduce) {
  .mobile-dialog-center-enter-active,
  .mobile-dialog-center-leave-active,
  .mobile-dialog-bottom-enter-active,
  .mobile-dialog-bottom-leave-active {
    transition: none !important;
  }

  .mobile-dialog-bottom-enter-active .mobile-dialog__panel--bottom,
  .mobile-dialog-bottom-leave-active .mobile-dialog__panel--bottom {
    transition: none !important;
  }
}
</style>
