<template>
  <!-- 移动端优化的按钮组件 — Orbit v3 -->
  <button
    :type="nativeType"
    :disabled="disabled || loading"
    :class="[
      'mobile-button',
      `mobile-button--${type}`,
      `mobile-button--${size}`,
      {
        'is-loading': loading,
        'is-disabled': disabled,
        'is-full-width': block,
      }
    ]"
    @click="handleClick"
  >
    <span
      v-if="loading"
      class="mobile-button__loading"
    >
      <SvgIcon
        class="loading-spinner"
        :icon="spinnerIcon"
      />
    </span>

    <span
      v-if="$slots.icon && !loading"
      class="mobile-button__icon"
    >
      <slot name="icon" />
    </span>

    <span
      v-if="$slots.default"
      class="mobile-button__content"
    >
      <slot />
    </span>
  </button>
</template>

<script setup>
import SvgIcon from './SvgIcon.vue'
import spinnerIcon from '../assets/icons/spinner.svg?raw'

const props = defineProps({
  type: {
    type: String,
    default: 'default',
    validator: (value) => ['default', 'primary', 'success', 'warning', 'danger'].includes(value)
  },
  size: {
    type: String,
    default: 'medium',
    validator: (value) => ['small', 'medium', 'large'].includes(value)
  },
  nativeType: {
    type: String,
    default: 'button'
  },
  disabled: {
    type: Boolean,
    default: false
  },
  loading: {
    type: Boolean,
    default: false
  },
  block: {
    type: Boolean,
    default: false
  }
})

const emit = defineEmits(['click'])

const handleClick = (e) => {
  if (!props.disabled && !props.loading) {
    emit('click', e)
  }
}
</script>

<style scoped>
.mobile-button {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  border: 1px solid var(--color-border-default, #ebebeb);
  border-radius: var(--radius-xl, 12px);
  font-family: var(--font-body);
  font-weight: 600;
  cursor: pointer;
  transition: all 0.2s ease;
  user-select: none;
  -webkit-user-select: none;
  -webkit-tap-highlight-color: transparent;
  touch-action: manipulation;
  position: relative;
  overflow: hidden;
}

/* 尺寸变体 */
.mobile-button--small {
  min-height: 34px;
  min-width: 34px;
  padding: 6px 14px;
  font-size: 13px;
  touch-action: manipulation;
}

@media (pointer: coarse) {
  .mobile-button--small::before {
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

.mobile-button--medium {
  min-height: 48px;
  min-width: 48px;
  padding: 12px 20px;
  font-size: 14px;
}

.mobile-button--large {
  min-height: 56px;
  min-width: 56px;
  padding: 14px 24px;
  font-size: 15px;
}

/* 全宽 */
.mobile-button.is-full-width {
  width: 100%;
}

/* 类型变体 — Orbit v3 色板 */
.mobile-button--default {
  background: var(--color-bg-surface, #f7f7f8);
  color: var(--color-text-primary, #0d0d0d);
  border-color: var(--color-border-default, #ebebeb);
}

.mobile-button--primary {
  background: var(--color-primary, #10a37f);
  color: var(--color-text-inverse, #fff);
  border-color: var(--color-primary, #10a37f);
}

.mobile-button--success {
  background: var(--color-primary, #10a37f);
  color: var(--color-text-inverse, #fff);
  border-color: var(--color-primary, #10a37f);
}

.mobile-button--warning {
  background: var(--color-warning, #f59e0b);
  color: var(--color-text-inverse, #fff);
  border-color: var(--color-warning, #f59e0b);
}

.mobile-button--danger {
  background: var(--color-danger, #ef4444);
  color: var(--color-text-inverse, #fff);
  border-color: var(--color-danger, #ef4444);
}

/* 状态 */
.mobile-button.is-disabled {
  opacity: 0.5;
  cursor: not-allowed;
  pointer-events: none;
}

.mobile-button.is-loading {
  cursor: wait;
  pointer-events: none;
  opacity: 0.7;
}

/* 悬停效果（仅桌面） */
@media (hover: hover) and (pointer: fine) {
  .mobile-button--default:hover {
    background: var(--color-bg-surface-hover, #ececec);
  }

  .mobile-button--primary:hover {
    background: var(--color-primary-hover, #0e8c6b);
  }

  .mobile-button--success:hover {
    background: var(--color-primary-hover, #0e8c6b);
  }

  .mobile-button--warning:hover {
    background: var(--color-warning-hover, #d97706);
  }

  .mobile-button--danger:hover {
    background: var(--color-danger-hover, #dc2626);
  }
}

/* 触摸反馈（移动端） */
@media (hover: none) and (pointer: coarse) {
  .mobile-button:active {
    transform: scale(0.98);
  }

  .mobile-button--default:active {
    background: var(--color-surface-hover);
  }

  .mobile-button--primary:active {
    background: var(--color-primary-active);
  }

  .mobile-button--success:active {
    background: var(--color-primary-active);
  }

  .mobile-button--warning:active {
    background: var(--color-warning-dark);
  }

  .mobile-button--danger:active {
    background: var(--color-danger-active);
  }
}

/* Loading 动画 */
.mobile-button__loading {
  display: inline-flex;
  align-items: center;
  justify-content: center;
}

.loading-spinner {
  width: 20px;
  height: 20px;
  animation: rotate 1.5s linear infinite;
}

@keyframes rotate {
  from { transform: rotate(0deg); }
  to { transform: rotate(360deg); }
}

/* 图标 */
.mobile-button__icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
}

/* 内容 */
.mobile-button__content {
  display: inline-flex;
  align-items: center;
  justify-content: center;
}

/* 深色模式 */
:root.dark .mobile-button--default,
.dark .mobile-button--default {
  background: var(--color-surface-muted, #27272a);
  color: var(--color-text, #f5f5f5);
  border-color: var(--color-border, #3f3f46);
}

:root.dark .mobile-button--default:hover,
.dark .mobile-button--default:hover {
  background: var(--color-surface-hover, #3f3f46);
}

:root.dark .mobile-button--primary:hover,
.dark .mobile-button--primary:hover {
  background: var(--color-primary-hover, #6ee7b7);
}

:root.dark .mobile-button--success:hover,
.dark .mobile-button--success:hover {
  background: var(--color-primary-hover, #6ee7b7);
}

:root.dark .mobile-button--default:active,
.dark .mobile-button--default:active {
  background: var(--color-surface-hover, #3f3f46);
}

/* 高对比度模式 */
@media (prefers-contrast: high) {
  .mobile-button {
    border-width: 2px;
  }
}

/* 减少动画 */
@media (prefers-reduced-motion: reduce) {
  .mobile-button,
  .loading-spinner {
    transition: none;
    animation: none;
  }

  .mobile-button:active {
    transform: none;
  }
}
</style>
