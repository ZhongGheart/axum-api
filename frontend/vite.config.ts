import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import { resolve } from 'path'
import compression from 'vite-plugin-compression'

// https://vite.dev/config/
export default defineConfig({
  plugins: [
    vue(),
    // 生产环境启用 gzip 压缩
    compression({
      algorithm: 'gzip',
      threshold: 10240, // 仅压缩大于 10KB 的文件
      deleteOriginFile: false,
    }),
  ],

  // 路径别名 @ → src/
  resolve: {
    alias: {
      '@': resolve(__dirname, 'src'),
    },
  },

  // 开发服务器配置
  server: {
    port: 3000,
    open: false,
    // 代理 API 请求到后端（开发环境）
    proxy: {
      '/api': {
        target: 'http://localhost:8080',
        changeOrigin: true,
      },
    },
  },

  // 构建配置
  build: {
    target: 'es2020',
    outDir: 'dist',
    assetsDir: 'assets',
    // 启用 CSS 代码分割
    cssCodeSplit: true,
    // 生成 sourcemap（生产环境关闭）
    sourcemap: false,
    // 块大小警告阈值（KB）
    chunkSizeWarningLimit: 500,
    // Rollup 打包选项
    rollupOptions: {
      output: {
        // 手动分包：将 node_modules 依赖拆分为 vendor 和 naive-ui
        manualChunks: {
          'vendor-vue': ['vue', 'vue-router', 'pinia'],
          'vendor-naive': ['naive-ui'],
          'vendor-axios': ['axios'],
        },
      },
    },
  },

  // CSS 预处理
  css: {
    preprocessorOptions: {
      scss: {},
    },
  },
})
