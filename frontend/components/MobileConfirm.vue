<template>
  <!-- 确认对话框：可组件化使用，也支持 Confirm.show 服务式调用 — Orbit v3 -->
  <Teleport to="body">
    <Transition
      name="mobile-confirm"
      @after-leave="emitAfterLeave"
    >
      <div
        v-if="modelValue"
        class="mobile-confirm"
        @click="onOverlayClick"
      >
        <div
          class="mobile-confirm__panel"
          role="dialog"
          aria-modal="true"
          @click.stop
        >
          <div
            v-if="icon"
            class="mobile-confirm__icon"
            :style="{ background: iconColor }"
          >
            <span class="mobile-confirm__icon-emoji">{{ icon }}</span>
          </div>

          <header
            v-if="title"
            class="mobile-confirm__header"
          >
            <div class="mobile-confirm__title">
              {{ title }}
            </div>
          </header>

          <div
            v-if="message"
            class="mobile-confirm__message"
          >
            {{ message }}
          </div>

          <div class="mobile-confirm__actions">
            <button
              class="mobile-confirm__btn mobile-confirm__btn--cancel"
              type="button"
              @click="onCancel"
            >
              {{ cancelTextComputed }}
            </button>
            <button
              class="mobile-confirm__btn"
              :class="confirmBtnClass"
              type="button"
              @click="onConfirm"
            >
              {{ confirmTextComputed }}
            </button>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<script>
import { computed } from 'vue'

const MobileConfirm = {
  name: 'MobileConfirm',
  props: {
    modelValue: { type: Boolean, default: false },
    title: { type: String, default: '' },
    message: { type: String, default: '' },
    confirmText: { type: String, default: '' },
    cancelText: { type: String, default: '' },
    icon: { type: String, default: '' },
    iconColor: { type: String, default: 'var(--color-danger-soft)' },
    type: {
      type: String,
      default: 'default',
      validator: (v) => ['default', 'danger'].includes(v)
    },
    closeOnClickOverlay: { type: Boolean, default: true }
  },
  emits: ['update:modelValue', 'confirm', 'cancel', 'after-leave'],
  setup(props, { emit }) {
    const confirmTextComputed = computed(() => props.confirmText || '确认')
    const cancelTextComputed = computed(() => props.cancelText || '取消')

    const confirmBtnClass = computed(() => {
      return props.type === 'danger' ? 'mobile-confirm__btn--danger' : 'mobile-confirm__btn--primary'
    })

    const close = () => emit('update:modelValue', false)

    const onConfirm = () => {
      emit('confirm')
      close()
    }

    const onCancel = () => {
      emit('cancel')
      close()
    }

    const onOverlayClick = () => {
      if (props.closeOnClickOverlay) onCancel()
    }

    const emitAfterLeave = () => emit('after-leave')

    return {
      confirmTextComputed,
      cancelTextComputed,
      confirmBtnClass,
      onConfirm,
      onCancel,
      onOverlayClick,
      emitAfterLeave
    }
  }
}

export default MobileConfirm

const buildNativeConfirmMessage = (options = {}) => {
  const parts = []
  if (options.title) parts.push(String(options.title))
  if (options.message) parts.push(String(options.message))
  return parts.join('\n\n') || '确认执行此操作？'
}

const getOrCreateThemeColorMeta = () => {
  let meta = document.querySelector('meta[name="theme-color"]')
  if (!meta) {
    meta = document.createElement('meta')
    meta.setAttribute('name', 'theme-color')
    document.head.appendChild(meta)
  }
  return meta
}

const applyNativeConfirmChromeTint = (color = '#8f8f99') => { // color-literal-ok: <meta theme-color> value
  if (typeof document === 'undefined') return () => {}

  const meta = getOrCreateThemeColorMeta()
  const previousColor = meta.getAttribute('content') || ''
  const previousScheme = document.documentElement.style.colorScheme

  meta.setAttribute('content', color)
  document.documentElement.style.colorScheme = 'light'

  return () => {
    if (previousColor) meta.setAttribute('content', previousColor)
    else meta.removeAttribute('content')
    document.documentElement.style.colorScheme = previousScheme
  }
}

export const Confirm = {
  /**
   * Confirm.show({ title, message, chromeColor }) -> Promise<boolean>
   * Uses the browser's native confirm dialog. On iOS Safari this is rendered
   * by the system as the native iOS confirmation alert.
   */
  show(options = {}) {
    if (typeof window === 'undefined' || typeof window.confirm !== 'function') {
      return Promise.resolve(false)
    }

    const restoreChromeTint = applyNativeConfirmChromeTint(options.chromeColor)
    try {
      return Promise.resolve(window.confirm(buildNativeConfirmMessage(options)))
    } finally {
      restoreChromeTint()
    }
  }
}
</script>

<style scoped>
.mobile-confirm {
  position: fixed;
  inset: 0;
  z-index: 1000;
  display: flex;
  align-items: center;
  justify-content: center;

  padding: 40px;
  background: var(--color-overlay-dialog);
  -webkit-tap-highlight-color: transparent;
}

.mobile-confirm__panel {
  width: 100%;
  max-width: 320px;
  border-radius: var(--radius-3xl, 18px);
  border: none;

  background: var(--color-surface);
  box-shadow: var(--shadow-dialog);

  overflow: hidden;
  padding: 24px;
}

.mobile-confirm__icon {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 48px;
  height: 48px;
  border-radius: 50%;
  margin: 0 auto 16px;
}

.mobile-confirm__icon-emoji {
  font-size: 24px;
  line-height: 1;
}

.mobile-confirm__header {
  padding: 0 0 6px;
}

.mobile-confirm__title {
  font-family: var(--font-body);
  font-size: 17px;
  font-weight: 600;
  color: var(--color-text-primary);
  text-align: center;
  line-height: 1.3;
}

.mobile-confirm__message {
  padding: 0 0 20px;
  font-family: var(--font-body);
  font-size: 14px;
  color: var(--color-text-secondary);
  text-align: center;
  white-space: pre-wrap;
  line-height: 1.5;
}

.mobile-confirm__actions {
  display: flex;
  gap: 8px;
}

.mobile-confirm__btn {
  flex: 1;
  padding: 12px;
  border-radius: var(--radius-lg, 10px);
  font-size: 14px;
  font-weight: 600;
  text-align: center;
  border: none;
  cursor: pointer;
  font-family: var(--font-body);
  -webkit-tap-highlight-color: transparent;
  transition: opacity 0.2s ease;
}

.mobile-confirm__btn--cancel {
  background: var(--color-bg-surface);
  color: var(--color-text-primary);
}

.mobile-confirm__btn--cancel:active {
  opacity: 0.7;
}

.mobile-confirm__btn--primary {
  background: var(--color-primary);
  color: var(--color-text-inverse);
}

.mobile-confirm__btn--primary:active {
  background: var(--color-primary-active);
}

.mobile-confirm__btn--danger {
  background: var(--color-danger);
  color: var(--color-text-inverse);
}

.mobile-confirm__btn--danger:active {
  background: var(--color-danger-hover);
}

/* 触摸反馈 */
@media (hover: none) and (pointer: coarse) {
  .mobile-confirm__btn:active {
    transform: scale(0.98);
  }
}

/* 深色模式 */
.dark .mobile-confirm {
  background: rgba(var(--rgb-black), 0.7);
}

.dark .mobile-confirm__panel {
  background: var(--color-surface);
  box-shadow: 0 20px 40px rgba(var(--rgb-black), 0.5);
}

.dark .mobile-confirm__title {
  color: var(--color-text);
}

.dark .mobile-confirm__message {
  color: var(--color-text-muted);
}

.dark .mobile-confirm__btn--cancel {
  background: var(--color-surface-muted);
  color: var(--color-text);
}

.dark .mobile-confirm__btn--primary:hover {
  background: var(--color-primary-hover);
}

/* 高对比度 */
@media (prefers-contrast: high) {
  .mobile-confirm__panel {
    border: 2px solid var(--color-border-default);
  }
}

/* 动画：Dialog 弹出 — Orbit v3: 200ms ease-out enter, 150ms ease-out leave */
.mobile-confirm-enter-active {
  transition: opacity 250ms ease-out, transform 200ms ease-out;
}

.mobile-confirm-leave-active {
  transition: opacity 200ms ease-in, transform 150ms ease-in;
}

.mobile-confirm-enter-from,
.mobile-confirm-leave-to {
  opacity: 0;
}

.mobile-confirm-enter-from .mobile-confirm__panel {
  transform: scale(0.95);
}

.mobile-confirm-leave-to .mobile-confirm__panel {
  transform: scale(0.95);
}

/* 减少动画 */
@media (prefers-reduced-motion: reduce) {
  .mobile-confirm-enter-active,
  .mobile-confirm-leave-active {
    transition: none;
  }
}
</style>
