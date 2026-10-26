<template>
  <!-- 移动端 Toast 组件：操作反馈提示（服务式调用，动态挂载到 body） -->
  <transition
    name="toast-slide"
    @before-enter="onBeforeEnter"
    @enter="onEnter"
    @after-enter="onAfterEnter"
    @before-leave="onBeforeLeave"
    @leave="onLeave"
    @after-leave="emit('close')"
  >
    <div
      v-if="visible"
      :class="[
        'mobile-toast',
        `mobile-toast--${type}`
      ]"
      role="alert"
      aria-live="polite"
    >
      <!-- 图标 -->
      <span class="mobile-toast__icon">
        <SvgIcon :icon="toastIcon" />
      </span>

      <!-- 内容 -->
      <span class="mobile-toast__message">
        {{ message }}
      </span>

      <!-- 关闭按钮（可点击手动关闭） -->
      <button
        v-if="closeable"
        class="mobile-toast__close"
        aria-label="关闭"
        @click="close"
      >
        <SvgIcon :icon="closeIcon" />
      </button>
    </div>
  </transition>
</template>

<script setup>
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import SvgIcon from './SvgIcon.vue'
import checkIcon from '../assets/icons/check.svg?raw'
import closeIcon from '../assets/icons/close.svg?raw'
import alertTriangleIcon from '../assets/icons/alert-triangle.svg?raw'
import infoCircleIcon from '../assets/icons/info-circle.svg?raw'

const props = defineProps({
  message: {
    type: String,
    required: true
  },
  type: {
    type: String,
    default: 'info',
    validator: (value) => ['info', 'success', 'warning', 'error'].includes(value)
  },
  duration: {
    type: Number,
    default: 2000
  },
  closeable: {
    type: Boolean,
    default: true
  }
})

const emit = defineEmits(['close'])

const visible = ref(false)
let timer = null

const toastIcon = computed(() => ({
  success: checkIcon,
  error: closeIcon,
  warning: alertTriangleIcon,
  info: infoCircleIcon
}[props.type] || infoCircleIcon))

const close = () => {
  visible.value = false
}

// 动画钩子
const onBeforeEnter = (el) => {
  el.style.opacity = '0'
  el.style.transform = 'translateY(-20px)'
}

const onEnter = (el, done) => {
  const duration = 300
  el.style.transition = `opacity ${duration}ms ease, transform ${duration}ms ease`
  // 强制重排以触发过渡
  requestAnimationFrame(() => {
    requestAnimationFrame(() => {
      el.style.opacity = '1'
      el.style.transform = 'translateY(0)'
      setTimeout(done, duration)
    })
  })
}

const onAfterEnter = () => {
  // 动画结束后启动自动消失定时器
  if (props.duration > 0) {
    timer = setTimeout(() => {
      close()
    }, props.duration)
  }
}

const onBeforeLeave = (el) => {
  el.style.opacity = '1'
  el.style.transform = 'translateY(0)'
}

const onLeave = (el, done) => {
  const duration = 200
  el.style.transition = `opacity ${duration}ms ease, transform ${duration}ms ease`
  requestAnimationFrame(() => {
    el.style.opacity = '0'
    el.style.transform = 'translateY(-20px)'
    setTimeout(done, duration)
  })
}

onMounted(() => {
  visible.value = true
})

onBeforeUnmount(() => {
  if (timer) {
    clearTimeout(timer)
  }
})

</script>

<script>
// ==================== Toast 服务 ====================
import {
  createApp,
  h as serviceH,
  nextTick,
  onBeforeUnmount as serviceOnBeforeUnmount,
  onMounted as serviceOnMounted,
  ref as serviceRef
} from 'vue'

let toastContainer = null
const toastInstances = []
let instanceIdCounter = 0

// 获取或创建容器
function getContainer() {
  if (!toastContainer) {
    toastContainer = document.createElement('div')
    toastContainer.className = 'mobile-toast-container'
    toastContainer.style.cssText = `
      position: fixed;
      top: max(env(safe-area-inset-top, 0px), 12px);
      left: 0;
      width: 100%;
      height: auto;
      display: flex;
      flex-direction: column;
      align-items: center;
      gap: var(--space-3);
      padding: 0 var(--space-4);
      box-sizing: border-box;
      pointer-events: none;
      z-index: 9999;
    `
    document.body.appendChild(toastContainer)
  }
  return toastContainer
}

// 创建 Toast 实例
function createToast(options) {
  const id = ++instanceIdCounter
  const container = getContainer()

  // 创建挂载点
  const mountPoint = document.createElement('div')
  mountPoint.style.cssText = `
    position: relative;
    pointer-events: auto;
    width: min(100%, 420px);
    max-width: 420px;
  `
  container.appendChild(mountPoint)

  const message = options.message || ''
  const isLongMessage = message.includes('\n') || message.length > 72
  const defaultDuration = isLongMessage ? 5200 : 2000

  // Toast 配置
  const toastOptions = {
    message,
    type: options.type || 'info',
    duration: options.duration !== undefined ? options.duration : defaultDuration,
    closeable: options.closeable !== undefined ? options.closeable : true
  }

  // 内联 Toast 组件定义（用于动态挂载）
  const MobileToastInline = {
    name: 'MobileToastInline',
    props: {
      message: String,
      type: String,
      duration: Number,
      closeable: Boolean
    },
    emits: ['close'],
    setup(props, { emit }) {
      const visible = serviceRef(false)
      const toastRef = serviceRef(null)
      let timer = null

      const animateIn = () => {
        const el = toastRef.value
        if (!el) return
        el.style.opacity = '0'
        el.style.transform = 'translateY(-20px)'
        el.style.transition = 'opacity 300ms ease, transform 300ms ease'
        requestAnimationFrame(() => {
          requestAnimationFrame(() => {
            el.style.opacity = '1'
            el.style.transform = 'translateY(0)'
          })
        })
      }

      const close = () => {
        const el = toastRef.value
        if (!el || !visible.value) {
          emit('close')
          return
        }

        visible.value = false
        if (timer) {
          clearTimeout(timer)
          timer = null
        }

        el.style.transition = 'opacity 200ms ease, transform 200ms ease'
        el.style.opacity = '0'
        el.style.transform = 'translateY(-20px)'
        window.setTimeout(() => emit('close'), 200)
      }

      serviceOnMounted(() => {
        visible.value = true
        nextTick(animateIn)
        if (props.duration > 0) {
          timer = setTimeout(() => {
            close()
          }, props.duration)
        }
      })

      serviceOnBeforeUnmount(() => {
        if (timer) clearTimeout(timer)
      })

      return () => {
        if (!visible.value) return null

        return serviceH('div', {
          ref: toastRef,
          class: ['mobile-toast', `mobile-toast--${props.type}`],
          role: 'alert',
          'aria-live': 'polite'
        }, [
          // 图标
          serviceH('span', { class: 'mobile-toast__icon' }, getIcon(props.type)),
          // 内容
          serviceH('span', { class: 'mobile-toast__message' }, props.message),
          // 关闭按钮
          props.closeable ? serviceH('button', {
            class: 'mobile-toast__close',
            onClick: close,
            'aria-label': '关闭'
          }, serviceH('svg', {
            viewBox: '0 0 24 24',
            fill: 'none',
            stroke: 'currentColor'
          }, serviceH('path', {
            'stroke-linecap': 'round',
            'stroke-linejoin': 'round',
            'stroke-width': '2',
            d: 'M6 18L18 6M6 6l12 12'
          }))) : null
        ])
      }
    }
  }

  // 创建应用
  const app = createApp({
    render() {
      return serviceH(MobileToastInline, {
        ...toastOptions,
        onClose: () => {
          // 关闭动画完成后卸载
          setTimeout(() => {
            destroyToast(id)
          }, 200)
        }
      })
    }
  })

  // 挂载
  app.mount(mountPoint)

  // 保存实例引用
  const toastInstance = {
    id,
    app,
    mountPoint,
    close: () => {
      const toastEl = mountPoint.querySelector('.mobile-toast')
      if (toastEl) {
        // 触发关闭
        toastEl.style.opacity = '0'
        toastEl.style.transform = 'translateY(-20px)'
        setTimeout(() => {
          destroyToast(id)
        }, 200)
      }
    }
  }

  toastInstances.push(toastInstance)

  return toastInstance
}

// 获取图标
function getIcon(type) {
  const icons = {
    success: serviceH('svg', { viewBox: '0 0 24 24', fill: 'none', stroke: 'currentColor' },
      serviceH('path', { 'stroke-linecap': 'round', 'stroke-linejoin': 'round', 'stroke-width': '2', d: 'M5 13l4 4L19 7' })
    ),
    error: serviceH('svg', { viewBox: '0 0 24 24', fill: 'none', stroke: 'currentColor' },
      serviceH('path', { 'stroke-linecap': 'round', 'stroke-linejoin': 'round', 'stroke-width': '2', d: 'M6 18L18 6M6 6l12 12' })
    ),
    warning: serviceH('svg', { viewBox: '0 0 24 24', fill: 'none', stroke: 'currentColor' },
      serviceH('path', { 'stroke-linecap': 'round', 'stroke-linejoin': 'round', 'stroke-width': '2', d: 'M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z' })
    ),
    info: serviceH('svg', { viewBox: '0 0 24 24', fill: 'none', stroke: 'currentColor' },
      serviceH('path', { 'stroke-linecap': 'round', 'stroke-linejoin': 'round', 'stroke-width': '2', d: 'M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z' })
    )
  }
  return icons[type] || icons.info
}

// 销毁 Toast 实例
function destroyToast(id) {
  const index = toastInstances.findIndex(t => t.id === id)
  if (index !== -1) {
    const instance = toastInstances[index]
    instance.app.unmount()
    instance.mountPoint.remove()
    toastInstances.splice(index, 1)

    // 如果没有实例了，移除容器
    if (toastInstances.length === 0 && toastContainer) {
      toastContainer.remove()
      toastContainer = null
    }
  }
}

// Toast 服务 API
const Toast = {
  // 显示 Toast
  show(options) {
    if (typeof options === 'string') {
      options = { message: options }
    }
    return createToast(options)
  },

  // 成功提示
  success(message, options = {}) {
    return Toast.show({
      message,
      type: 'success',
      ...options
    })
  },

  // 错误提示
  error(message, options = {}) {
    return Toast.show({
      message,
      type: 'error',
      duration: 3000, // 错误提示显示更长时间
      ...options
    })
  },

  // 警告提示
  warning(message, options = {}) {
    return Toast.show({
      message,
      type: 'warning',
      duration: 3000,
      ...options
    })
  },

  // 信息提示
  info(message, options = {}) {
    return Toast.show({
      message,
      type: 'info',
      ...options
    })
  },

  // 关闭所有 Toast
  closeAll() {
    [...toastInstances].forEach(instance => {
      instance.close()
    })
  }
}

// Toast 服务导出（组件 default export 由 <script setup> 自动处理）
export { Toast }
</script>

<style>
.mobile-toast {
  position: relative;
  display: flex;
  align-items: flex-start;
  gap: var(--space-2);
  width: 100%;
  min-width: 0;
  max-width: 100%;
  max-height: min(60vh, 420px);
  padding: var(--space-3) var(--space-4);
  background: rgba(var(--rgb-white), 0.9);
  backdrop-filter: blur(20px);
  -webkit-backdrop-filter: blur(20px);
  border-radius: var(--radius-card);
  border: 1px solid var(--separator);
  box-shadow: var(--shadow-elevated-hover);
  font-family: var(--font-body);
  font-size: var(--font-size-sm-mobile);
  color: var(--text-primary);
  user-select: none;
  -webkit-tap-highlight-color: transparent;
  overflow: hidden;
}

/* 图标 */
.mobile-toast__icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  width: var(--size-icon-md);
  height: var(--size-icon-md);
  margin-top: 2px;
}

.mobile-toast__icon svg {
  width: 100%;
  height: 100%;
}

/* 类型变体 - 图标颜色 */
.mobile-toast--info .mobile-toast__icon {
  color: var(--accent-blue);
}

.mobile-toast--success .mobile-toast__icon {
  color: var(--accent-green);
}

.mobile-toast--warning .mobile-toast__icon {
  color: var(--accent-amber);
}

.mobile-toast--error .mobile-toast__icon {
  color: var(--accent-red);
}

/* 消息内容 */
.mobile-toast__message {
  flex: 1;
  min-width: 0;
  max-height: calc(min(60vh, 420px) - 24px);
  overflow-y: auto;
  line-height: 1.5;
  white-space: pre-line;
  word-break: break-word;
  overflow-wrap: anywhere;
  padding-right: var(--space-1);
}

/* 关闭按钮 */
.mobile-toast__close {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  align-self: flex-start;
  width: var(--size-icon-md);
  height: var(--size-icon-md);
  margin-left: var(--space-1);
  padding: 0;
  border: none;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  border-radius: var(--radius-control);
  transition: var(--transition-default);
  -webkit-tap-highlight-color: transparent;
}

.mobile-toast__close svg {
  width: var(--size-icon-sm);
  height: var(--size-icon-sm);
}

.mobile-toast__close:hover {
  color: var(--text-primary);
  background: var(--color-surface-muted);
}

.mobile-toast__close:active {
  transform: scale(0.95);
}

/* 深色模式 */
.dark .mobile-toast {
  background: rgba(var(--rgb-neutral-900), 0.9);
  border-color: rgba(var(--rgb-white), 0.1);
  color: var(--text-primary);
}

.dark .mobile-toast__close {
  color: var(--text-secondary);
}

.dark .mobile-toast__close:hover {
  color: var(--text-primary);
  background: rgba(var(--rgb-white), 0.1);
}

/* 高对比度模式 */
@media (prefers-contrast: high) {
  .mobile-toast {
    border-width: 2px;
    font-weight: 500;
  }

  .mobile-toast__close {
    border: 1px solid var(--separator);
  }
}

/* 减少动画 */
@media (prefers-reduced-motion: reduce) {
  .mobile-toast {
    transition: none;
  }

  .mobile-toast__close {
    transition: none;
  }
}
</style>
