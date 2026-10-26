<template>
  <!-- 移动端标签组件：状态标签、数量角标 -->
  <span
    :class="[
      'mobile-tag',
      `mobile-tag--${type}`,
      `mobile-tag--${size}`,
      { 'mobile-tag--round': round }
    ]"
  >
    <slot />
  </span>
</template>

<script setup>
defineProps({
  type: {
    type: String,
    default: 'default',
    validator: (value) => ['default', 'primary', 'success', 'warning', 'danger'].includes(value)
  },
  size: {
    type: String,
    default: 'medium',
    validator: (value) => ['small', 'medium'].includes(value)
  },
  round: {
    type: Boolean,
    default: false
  }
})
</script>

<style scoped>
.mobile-tag {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-family: var(--font-body);
  font-weight: 500;
  white-space: nowrap;
  border-radius: var(--radius-control);
  padding: 0 var(--space-2-5);
  line-height: 1;
  -webkit-tap-highlight-color: transparent;
  user-select: none;
  transition: var(--transition-default);
  cursor: default;
}

/* 触摸反馈（可点击时由父组件控制，内联元素本身不需要 :active） */

/* 尺寸变体 */
.mobile-tag--small {
  height: 20px;
  font-size: var(--font-size-xs-mobile);
  gap: var(--space-0-5);
}

.mobile-tag--medium {
  height: 24px;
  font-size: var(--font-size-sm-mobile);
  gap: var(--space-1);
}

/* 圆角胶囊 */
.mobile-tag--round {
  border-radius: var(--radius-full);
}

/* 类型变体 */
.mobile-tag--default {
  background: var(--color-surface-muted);
  color: var(--text-secondary);
  border: 1px solid var(--separator);
}

.mobile-tag--primary {
  background: var(--accent-blue-fill-soft);
  color: var(--accent-blue);
  border: 1px solid var(--accent-blue-border-soft);
}

.mobile-tag--success {
  background: var(--color-success-soft);
  color: var(--accent-green);
  border: 1px solid var(--color-success-border-soft);
}

.mobile-tag--warning {
  background: var(--color-warning-soft);
  color: var(--accent-amber);
  border: 1px solid var(--color-warning-border-soft);
}

.mobile-tag--danger {
  background: var(--color-danger-soft);
  color: var(--accent-red);
  border: 1px solid var(--color-danger-border-soft);
}

/* 深色模式 */
.dark .mobile-tag--default {
  background: rgba(var(--rgb-white), 0.08);
  color: var(--text-secondary);
  border-color: rgba(var(--rgb-white), 0.12);
}

.dark .mobile-tag--primary {
  background: rgba(var(--rgb-ios-blue), 0.15);
}

.dark .mobile-tag--success {
  background: rgba(var(--rgb-ios-green), 0.15);
}

.dark .mobile-tag--warning {
  background: rgba(var(--rgb-ios-orange), 0.15);
}

.dark .mobile-tag--danger {
  background: rgba(var(--rgb-ios-red), 0.15);
}

/* 高对比度模式 */
@media (prefers-contrast: high) {
  .mobile-tag {
    border-width: 2px;
    font-weight: 600;
  }
}

/* 减少动画 */
@media (prefers-reduced-motion: reduce) {
  .mobile-tag {
    transition: none;
  }
}
</style>
