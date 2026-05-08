/**
 * 应用入口
 *
 * 注册 Pinia、Router、Naive UI 全局组件、自定义指令。
 */
import { createApp } from 'vue'
import naive from 'naive-ui'
import App from './App.vue'
import router from './router'
import pinia from './stores'
import { vPermission } from './directives/permission'
import './assets/styles/global.css'

const app = createApp(App)

app.use(pinia)
app.use(router)
app.use(naive)

// 注册全局权限指令 v-permission
app.directive('permission', vPermission)

app.mount('#app')
