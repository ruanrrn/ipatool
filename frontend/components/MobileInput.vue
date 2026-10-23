<template>
  <!-- 移动端优化的输入框组件 — Orbit v3 -->
  <div
    :class="[
      'mobile-input',
      {
        'is-focused': isFocused,
        'is-disabled': disabled,
        'is-readonly': readonly,
        'has-error': error,
        'has-value': hasValue
      }
    ]"
  >
    <label
      v-if="label"
      class="mobile-input__label"
      :for="inputId"
    >
      {{ label }}
      <span
        v-if="required"
        class="mobile-input__required"
      >*</span>
    </label>

    <div class="mobile-input__wrapper">
      <span
        v-if="$slots.prefix"
        class="mobile-input__prefix"
      >
        <slot name="prefix" />
      </span>

      <input
        :id="inputId"
        ref="inputRef"
        :type="currentType"
        :value="modelValue"
        :placeholder="placeholder"
        :disabled="disabled"
        :readonly="readonly"
        :autocomplete="autocomplete"
        :maxlength="maxlength"
        :inputmode="inputmode"
        class="mobile-input__field"
        @input="handleInput"
        @focus="handleFocus"
        @blur="handleBlur"
        @change="handleChange"
        @keyup="handleKeyup"
      >

      <span
        v-if="$slots.suffix"
        class="mobile-input__suffix"
      >
        <slot name="suffix" />
      </span>

      <!-- 密码眼睛按钮 -->
      <button
        v-if="type === 'password' && hasValue && !disabled && !readonly"
        class="mobile-input__eye"
        type="button"
        :aria-label="passwordVisible ? '隐藏密码' : '显示密码'"
        @click="togglePassword"
      >
        <SvgIcon
          v-if="!passwordVisible"
          class="mobile-input__icon mobile-input__icon--eye"
          :icon="eyeIcon"
        />
        <SvgIcon
          v-else
          class="mobile-input__icon mobile-input__icon--eye"
          :icon="eyeOffIcon"
        />
      </button>

      <!-- 清除按钮 -->
      <button
        v-else-if="clearable && hasValue && !disabled && !readonly"
        class="mobile-input__clear"
        type="button"
        aria-label="清空输入"
        @click="handleClear"
      >
        <SvgIcon
          class="mobile-input__icon mobile-input__icon--clear"
          :icon="closeIcon"
        />
      </button>
    </div>

    <p
      v-if="error"
      class="mobile-input__error"
    >
      {{ error }}
    </p>

    <p
      v-if="hint && !error"
      class="mobile-input__hint"
    >
      {{ hint }}
    </p>
  </div>
</template>

<script setup>
import { ref, computed } from 'vue'
import { useId } from 'vue'
import SvgIcon from './SvgIcon.vue'
import eyeIcon from '../assets/icons/eye.svg?raw'
import eyeOffIcon from '../assets/icons/eye-off.svg?raw'
import closeIcon from '../assets/icons/close.svg?raw'

const props = defineProps({
  modelValue: {
    type: [String, Number],
    default: ''
  },
  type: {
    type: String,
    default: 'text',
    validator: (value) => ['text', 'password', 'email', 'tel', 'url', 'number'].includes(value)
  },
  label: {
    type: String,
    default: ''
  },
  placeholder: {
    type: String,
    default: ''
  },
  disabled: {
    type: Boolean,
    default: false
  },
  readonly: {
    type: Boolean,
    default: false
  },
  required: {
    type: Boolean,
    default: false
  },
  clearable: {
    type: Boolean,
    default: false
  },
  error: {
    type: String,
    default: ''
  },
  hint: {
    type: String,
    default: ''
  },
  autocomplete: {
    type: String,
    default: 'off'
  },
  maxlength: {
    type: [String, Number],
    default: null
  },
  inputmode: {
    type: String,
    default: 'text'
  }
})

const emit = defineEmits(['update:modelValue', 'focus', 'blur', 'change', 'clear', 'keyup'])

const inputId = useId()
const inputRef = ref(null)
const isFocused = ref(false)
const passwordVisible = ref(false)

const hasValue = computed(() => {
  return props.modelValue !== '' && props.modelValue !== null && props.modelValue !== undefined
})

const currentType = computed(() => {
  if (props.type === 'password') {
    return passwordVisible.value ? 'text' : 'password'
  }
  return props.type
})

const handleInput = (e) => {
  const value = props.type === 'number' ? parseFloat(e.target.value) : e.target.value
  emit('update:modelValue', value)
}

const handleFocus = (e) => {
  isFocused.value = true
  emit('focus', e)
  // Defer until iOS Safari has resized the visual viewport for the keyboard
  if (typeof window !== 'undefined' && 'visualViewport' in window) {
    setTimeout(() => {
      try {
        e.target?.scrollIntoView({ block: 'nearest', behavior: 'auto' })
      } catch {
        // ignore
      }
    }, 320)
  }
}

const handleBlur = (e) => {
  isFocused.value = false
  emit('blur', e)
}

const handleChange = (e) => {
  const value = props.type === 'number' ? parseFloat(e.target.value) : e.target.value
  emit('change', value)
}

const handleKeyup = (e) => {
  emit('keyup', e)
}

const handleClear = () => {
  emit('update:modelValue', '')
  emit('clear')
  inputRef.value?.focus()
}

const togglePassword = () => {
  passwordVisible.value = !passwordVisible.value
}

// 暴露方法
defineExpose({
  focus: () => inputRef.value?.focus(),
  blur: () => inputRef.value?.blur()
})
</script>

<style scoped>
.mobile-input {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.mobile-input__label {
  display: flex;
  align-items: center;
  gap: 2px;
  font-size: 12px;
  font-weight: 500;
  color: var(--color-text-secondary, #6e6e80);
}

.mobile-input__required {
  color: var(--color-danger, #ef4444);
  font-weight: 600;
}

.mobile-input__wrapper {
  position: relative;
  display: flex;
  align-items: center;
  gap: 8px;
  min-height: 48px;
  padding: 14px 12px;
  background: var(--color-bg-surface, #f7f7f8);
  border: 1px solid var(--color-border-default, #ebebeb);
  border-radius: 12px;
  transition: all 0.2s ease;
}

.mobile-input__wrapper.is-focused {
  background: var(--color-surface, #fff);
  border: 2px solid var(--color-primary, #10a37f);
  box-shadow: 0 0 0 3px rgba(16, 163, 127, 0.1);
}

.mobile-input__wrapper.has-error {
  background-color: var(--color-danger-soft);
  border: 1px solid var(--color-danger);
  box-shadow: 0 0 0 3px rgba(239, 68, 68, 0.1);
}

.mobile-input__wrapper.has-error.is-focused {
  background-color: var(--color-danger-soft);
  border: 2px solid var(--color-danger);
  box-shadow: 0 0 0 3px rgba(239, 68, 68, 0.1);
}

.mobile-input__wrapper.is-disabled {
  background: var(--color-bg-page, #f0f0f0);
  color: var(--color-text-disabled, #d1d5db);
  cursor: not-allowed;
  opacity: 0.5;
}

.mobile-input__wrapper.is-readonly {
  background: rgba(0, 0, 0, 0.02);
  cursor: default;
}

.mobile-input__prefix,
.mobile-input__suffix {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  color: var(--color-text-secondary, #6e6e80);
  font-size: 15px;
}

.mobile-input__wrapper.is-focused .mobile-input__prefix {
  color: var(--color-primary, #10a37f);
}

.mobile-input__field {
  flex: 1;
  min-width: 0;
  background: transparent;
  border: none;
  outline: none;
  font-family: var(--font-body);
  /* 16px to prevent iOS Safari focus auto-zoom */
  font-size: 16px;
  color: var(--color-text-primary, #0d0d0d);
  line-height: 1.5;
  width: 100%;
}

.mobile-input__field::placeholder {
  color: var(--color-text-tertiary, #c0c0c0);
}

.mobile-input__field:disabled {
  color: var(--color-text-disabled, #d1d5db);
  cursor: not-allowed;
}

.mobile-input__field:-webkit-autofill,
.mobile-input__field:-webkit-autofill:hover,
.mobile-input__field:-webkit-autofill:focus {
  -webkit-text-fill-color: var(--color-text-primary, #0d0d0d);
  -webkit-box-shadow: 0 0 0 48px var(--color-bg-surface, #f7f7f8) inset;
  transition: background-color 5000s ease-in-out 0s;
}

/* 图标尺寸，直接作用在 SvgIcon 包裹层，避免 raw SVG 在按钮内被撑大 */
.mobile-input__icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  line-height: 0;
}

.mobile-input__icon--clear {
  width: 10px;
  height: 10px;
}

.mobile-input__icon--eye {
  width: 18px;
  height: 18px;
}

/* 清除按钮 — Orbit v3: 18x18 圆形，避免在搜索框中显得过大 */
.mobile-input__clear {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 18px;
  min-width: 18px;
  min-height: 18px;
  padding: 0;
  background: var(--color-border-subtle, #d1d5db);
  border: none;
  border-radius: 50%;
  color: var(--color-text-inverse, #fff);
  cursor: pointer;
  transition: all 0.2s ease;
  flex-shrink: 0;
  overflow: hidden;
}

.mobile-input__clear:active {
  transform: scale(0.9);
  background: var(--color-text-tertiary, #c0c0c0);
}

/* 密码眼睛按钮 */
.mobile-input__eye {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  padding: 0;
  background: transparent;
  border: none;
  color: var(--color-text-secondary, #6e6e80);
  cursor: pointer;
  transition: color 0.2s ease;
  flex-shrink: 0;
  -webkit-tap-highlight-color: transparent;
}

.mobile-input__eye:active {
  color: var(--color-text-primary, #0d0d0d);
}

.mobile-input__eye svg {
  width: 18px;
  height: 18px;
}

.mobile-input__error,
.mobile-input__hint {
  font-size: 12px;
  line-height: 1.4;
  padding: 2px 0;
}

.mobile-input__error {
  color: var(--color-danger, #ef4444);
}

.mobile-input__hint {
  color: var(--color-text-tertiary, #c0c0c0);
}

/* 深色模式 — .dark 在 <html> 上，scoped 会自动将属性加到末尾选择器 */
.dark .mobile-input__wrapper {
  background: var(--color-surface-muted, #27272a) !important;
  border-color: var(--color-border, #3f3f46);
}

.dark .mobile-input__wrapper.is-focused {
  background: var(--color-surface, #18181b) !important;
  border-color: var(--color-primary, #34d399);
  box-shadow: 0 0 0 3px rgba(16, 163, 127, 0.15);
}

.dark .mobile-input__wrapper.has-error {
  background-color: rgba(239, 68, 68, 0.1) !important;
  border-color: var(--color-danger, #ef4444);
  box-shadow: 0 0 0 3px rgba(239, 68, 68, 0.15);
}

.dark .mobile-input__wrapper.has-error.is-focused {
  background-color: rgba(239, 68, 68, 0.1) !important;
  border-color: var(--color-danger, #ef4444);
  box-shadow: 0 0 0 3px rgba(239, 68, 68, 0.15);
}

.dark .mobile-input__wrapper.is-disabled {
  background: var(--color-bg, #09090b) !important;
}

.dark .mobile-input__wrapper.is-readonly {
  background: rgba(255, 255, 255, 0.01) !important;
}

.dark .mobile-input__field {
  color: var(--color-text, #f5f5f5) !important;
}

.dark .mobile-input__field::placeholder {
  color: var(--color-text-tertiary, #71717a);
}

.dark .mobile-input__field:-webkit-autofill,
.dark .mobile-input__field:-webkit-autofill:hover,
.dark .mobile-input__field:-webkit-autofill:focus {
  -webkit-box-shadow: 0 0 0 48px var(--color-surface, #18181b) inset;
}

.dark .mobile-input__clear {
  background: var(--color-border-strong, #52525b);
  color: var(--color-text-inverse, #0d0d0d);
}

.dark .mobile-input__eye {
  color: var(--color-text-muted, #a1a1aa);
}

.dark .mobile-input__label {
  color: var(--color-text-muted, #a1a1aa);
}

.dark .mobile-input__prefix,
.dark .mobile-input__suffix {
  color: var(--color-text-muted, #a1a1aa);
}

.dark .mobile-input__hint {
  color: var(--color-text-tertiary, #71717a);
}

/* 高对比度模式 */
@media (prefers-contrast: high) {
  .mobile-input__wrapper {
    border-width: 2px;
  }

  .mobile-input__label {
    font-weight: 600;
  }
}

/* 减少动画 */
@media (prefers-reduced-motion: reduce) {
  .mobile-input__wrapper,
  .mobile-input__clear,
  .mobile-input__eye {
    transition: none;
  }

  .mobile-input__clear:active {
    transform: none;
  }
}

/* 触摸优化 */
@media (hover: none) and (pointer: coarse) {
  .mobile-input__clear:active {
    transform: scale(0.9);
  }
}
</style>
