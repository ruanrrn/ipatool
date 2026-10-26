<template>
  <!-- Loading skeleton mode -->
  <div
    v-if="type === 'loading'"
    class="empty-state"
  >
    <div class="skeleton skeleton--line skeleton--w80" />
    <div class="skeleton skeleton--line skeleton--w60" />
    <div class="skeleton skeleton--line skeleton--w70" />
  </div>

  <!-- Empty state mode -->
  <div
    v-else
    class="empty-state"
  >
    <div class="empty-state__icon">
      <SvgIcon
        class="empty-state__icon-svg"
        :icon="emptyPackageIcon"
      />
    </div>
    <p class="empty-state__text">
      {{ text || '暂无数据' }}
    </p>
  </div>
</template>

<script setup>
import SvgIcon from './SvgIcon.vue'
import emptyPackageIcon from '../assets/icons/empty-package.svg?raw'

defineProps({
  type: {
    type: String,
    default: 'empty',
    validator: v => ['empty', 'loading'].includes(v)
  },
  text: {
    type: String,
    default: ''
  }
})
</script>

<style scoped>
.empty-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  width: 100%;
  height: 100%;
  padding: 48px 24px;
  text-align: center;
  background: transparent;
}

/* Skeleton */
@keyframes skeleton-pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.4; }
}
.skeleton {
  background: var(--color-surface-muted);
  border-radius: 8px;
  animation: skeleton-pulse 1.5s ease-in-out infinite;
  margin-bottom: 10px;
}
.dark .skeleton {
  background: var(--color-surface-muted);
}
.skeleton--line {
  height: 16px;
}
.skeleton--w80 {
  width: 80%;
}
.skeleton--w60 {
  width: 60%;
}
.skeleton--w70 {
  width: 70%;
}

/* Empty */
.empty-state__icon {
  margin-bottom: 16px;
  opacity: 0.4;
  color: var(--color-text-muted);
}
.empty-state__icon-svg {
  width: 48px;
  height: 48px;
}
.dark .empty-state__icon {
  color: var(--color-text-muted);
}
.empty-state__text {
  font-size: 13px;
  color: var(--color-text-tertiary);
  margin: 0;
}
.dark .empty-state__text {
  color: var(--color-text-tertiary);
}
</style>
