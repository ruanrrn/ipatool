<template>
  <MobileDialog
    :model-value="visible"
    title="发布到 GitHub"
    @update:model-value="emit('update:visible', $event)"
  >
    <div class="space-y-3">
      <div class="text-sm text-txt-secondary dark:text-txt-dark-secondary">
        将 <strong class="text-txt dark:text-txt-dark">{{ appName }}</strong> 发布到 GitHub 仓库
      </div>
      <div
        v-if="warnings.length"
        class="text-sm text-yellow-600 dark:text-yellow-400 space-y-1"
      >
        <div
          v-for="(w, i) in warnings"
          :key="i"
        >
          ⚠️ {{ w }}
        </div>
      </div>
      <div class="flex flex-col gap-1">
        <label class="text-sm text-txt dark:text-txt-dark">备注（每行一条）</label>
        <textarea
          :value="notes"
          rows="3"
          class="w-full rounded-xl border border-border dark:border-border-dark bg-bg dark:bg-bg-dark px-3 py-2 text-[16px] text-txt dark:text-txt-dark focus:outline-none focus:ring-1 focus:ring-primary"
          placeholder="可选备注..."
          @input="emit('update:notes', $event.target.value)"
        />
      </div>
      <div
        v-if="result"
        class="text-sm"
        :class="result.ok ? 'text-brand' : 'text-danger'"
      >
        {{ result.msg }}
      </div>
    </div>
    <template #footer>
      <div class="flex gap-2 justify-end">
        <MobileButton
          size="small"
          @click="emit('update:visible', false)"
        >
          取消
        </MobileButton>
        <MobileButton
          type="primary"
          size="small"
          :loading="loading"
          @click="emit('publish')"
        >
          确认发布
        </MobileButton>
      </div>
    </template>
  </MobileDialog>
</template>

<script setup>
import MobileButton from './MobileButton.vue'
import MobileDialog from './MobileDialog.vue'

defineProps({
  visible: {
    type: Boolean,
    default: false
  },
  appName: {
    type: String,
    default: ''
  },
  notes: {
    type: String,
    default: ''
  },
  warnings: {
    type: Array,
    default: () => []
  },
  loading: {
    type: Boolean,
    default: false
  },
  result: {
    type: Object,
    default: null
  }
})

const emit = defineEmits(['update:visible', 'update:notes', 'publish'])
</script>
