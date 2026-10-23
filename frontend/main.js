import { createApp } from 'vue'
import { createPinia } from 'pinia'

// Design tokens (Radix Colors)
import '@radix-ui/colors/slate.css'
import '@radix-ui/colors/slate-dark.css'
import '@radix-ui/colors/blue.css'
import '@radix-ui/colors/blue-dark.css'
import '@radix-ui/colors/red.css'
import '@radix-ui/colors/red-dark.css'
import '@radix-ui/colors/amber.css'
import '@radix-ui/colors/amber-dark.css'
import '@radix-ui/colors/grass.css'
import '@radix-ui/colors/grass-dark.css'

import App from './App.vue'
import './tokens.css'
import './style.css'

const app = createApp(App)
const pinia = createPinia()

// Prevent iOS Safari pinch-to-zoom / multi-touch gesture zoom
if (typeof document !== 'undefined') {
  document.addEventListener(
    'gesturestart',
    (e) => {
      e.preventDefault()
    },
    { passive: false }
  )
  document.addEventListener(
    'gesturechange',
    (e) => {
      e.preventDefault()
    },
    { passive: false }
  )
}

app.use(pinia)
app.mount('#app')
