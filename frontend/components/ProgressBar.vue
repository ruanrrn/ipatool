<template>
  <div
    class="progress-bar"
    :class="[size === 'mini' ? 'h-[3px]' : 'h-[4px]']"
  >
    <div
      class="progress-bar__fill"
      :style="{ width: clampedPercent + '%', backgroundColor: color }"
    />
  </div>
</template>

<script setup>
import { computed } from 'vue'

const props = defineProps({
  percent: {
    type: Number,
    default: 0
  },
  color: {
    type: String,
    default: 'var(--color-primary)'
  },
  size: {
    type: String,
    default: 'default',
    validator: v => ['mini', 'default'].includes(v)
  }
})

const clampedPercent = computed(() => Math.min(100, Math.max(0, props.percent)))
</script>

<style scoped>
.progress-bar {
  width: 100%;
  background: var(--color-border);
  border-radius: 2px;
  overflow: hidden;
}
.dark .progress-bar {
  background: var(--color-surface-muted);
}
.progress-bar__fill {
  height: 100%;
  border-radius: 2px;
  transition: width 0.3s ease;
}
</style>
