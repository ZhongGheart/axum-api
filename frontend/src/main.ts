/**
 * 应用入口
 *
 * 注册 Pinia、Router、Naive UI 全局组件。
 */
import { createApp } from 'vue'
import naive from 'naive-ui'
import App from './App.vue'
import router from './router'
import pinia from './stores'
import './assets/styles/global.css'

const app = createApp(App)

app.use(pinia)
app.use(router)
app.use(naive)

app.mount('#app')
