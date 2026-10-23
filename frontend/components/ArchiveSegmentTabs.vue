<template>
  <div class="archive-segment">
    <button
      class="archive-seg"
      :class="{ active: modelValue === 'favorites' }"
      @click="emit('update:modelValue', 'favorites')"
    >
      收藏 ({{ favoritesCount }})
    </button>
    <button
      class="archive-seg"
      :class="{ active: modelValue === 'delisted' }"
      @click="emit('update:modelValue', 'delisted')"
    >
      已下架 ({{ delistedCount }})
    </button>
  </div>
</template>

<script setup>
defineProps({
  modelValue: {
    type: String,
    default: 'favorites'
  },
  favoritesCount: {
    type: Number,
    default: 0
  },
  delistedCount: {
    type: Number,
    default: 0
  }
})

const emit = defineEmits(['update:modelValue'])
</script>

<style scoped>
.archive-segment {
 display: flex;
 gap: 0;
 background: var(--color-surface-muted, #f7f7f8);
 border-radius: var(--radius-xl);
 padding: 3px;
 margin-bottom: 0;
}
.dark .archive-segment {
 background: var(--color-surface, #18181b);
}

.archive-seg {
 flex: 1;
 padding: 9px;
 text-align: center;
 font-size: var(--font-size-label);
 font-weight: 500;
 border-radius: var(--radius-lg);
 color: var(--color-text-muted, #6e6e80);
 border: none;
 background: transparent;
 cursor: pointer;
 transition: all 0.2s ease;
 -webkit-tap-highlight-color: transparent;
 touch-action: manipulation;
 user-select: none;
 -webkit-user-select: none;
}
.dark .archive-seg {
 color: var(--color-text-muted, #a1a1aa);
}

.archive-seg.active {
 background: var(--color-surface, #fff);
 color: var(--color-text, #0d0d0d);
 box-shadow: 0 1px 3px rgba(0, 0, 0, 0.08);
}
.dark .archive-seg.active {
 background: var(--color-surface-muted, #27272a);
 color: var(--color-text, #f5f5f5);
 box-shadow: 0 1px 3px rgba(0, 0, 0, 0.3);
}
</style>
