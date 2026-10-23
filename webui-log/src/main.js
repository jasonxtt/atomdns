import { createApp } from 'vue'
import App from './App.vue'
import { pageComponent } from './pages'
import { createAppRouter } from './router'
import { createThemeController } from './utils/theme'
import './style.css'

let storage = null
try { storage = window.localStorage } catch { /* blocked storage: the theme follows the system */ }
const theme = createThemeController({ root: document.documentElement, storage, matchMedia: window.matchMedia?.bind(window) })

createApp(App)
  .use(createAppRouter({ pageComponent }))
  .provide('theme', theme)
  .mount('#app')
