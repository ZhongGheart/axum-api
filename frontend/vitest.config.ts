import { defineConfig } from 'vitest/config'
import { resolve } from 'path'
import vue from '@vitejs/plugin-vue'

export default defineConfig({
  // 路由表里有懒加载的 .vue 组件：不注册这个插件，导航一旦成立就会在
  // import-analysis 阶段炸掉（"content contains invalid JS syntax"）。
  plugins: [vue()],
  resolve: {
    alias: { '@': resolve(__dirname, 'src') },
  },
  test: {
    environment: 'jsdom',
    include: ['src/**/*.spec.ts'],
    restoreMocks: true,
  },
})
