import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import { resolve } from 'path'
import compression from 'vite-plugin-compression'

// https://vite.dev/config/
export default defineConfig({
  plugins: [
    vue(),

    // 生产环境启用 gzip + brotli 双重压缩
    compression({
      algorithm: 'gzip',
      threshold: 10240,
      deleteOriginFile: false,
    }),
    compression({
      algorithm: 'brotliCompress',
      threshold: 10240,
      deleteOriginFile: false,
      compressionOptions: { level: 11 },
    }),
  ],

  resolve: {
    alias: {
      '@': resolve(__dirname, 'src'),
    },
  },

  server: {
    port: 3000,
    open: false,
    proxy: {
      '/api': {
        target: 'http://localhost:9527',
        changeOrigin: true,
      },
    },
  },

  build: {
    target: 'es2020',
    outDir: 'dist',
    assetsDir: 'assets',
    cssCodeSplit: true,
    sourcemap: false,
    chunkSizeWarningLimit: 500,
    cssMinify: 'esbuild',
    modulePreload: true,
    rollupOptions: {
      output: {
        // 更精细的手动分包 + 命名策略
        manualChunks(id: string) {
          // 将 echarts 单独打包（按需加载减少首屏体积）
          if (id.includes('echarts')) return 'vendor-echarts'
          if (id.includes('naive-ui')) return 'vendor-naive'
          if (id.includes('vue-echarts')) return 'vendor-echarts'
          if (id.includes('vue-router')) return 'vendor-vue'
          if (id.includes('pinia')) return 'vendor-vue'
          if (id.includes('vue')) return 'vendor-vue'
          if (id.includes('axios')) return 'vendor-axios'
          if (id.includes('@vicons')) return 'vendor-icons'
          if (id.includes('node_modules')) return 'vendor-other'
          // 业务组件按路由懒加载自动拆分
        },
        // 稳定 hash
        entryFileNames: 'assets/[name]-[hash:8].js',
        chunkFileNames: 'assets/[name]-[hash:8].js',
        assetFileNames: 'assets/[name]-[hash:8][extname]',
      },
    },
    // 告知 Rollup 哪些模块可以外部化（CDN 引入）
    // 实际生产使用 CDN 时取消注释以下块并安装 vite-plugin-cdn-import
    // rollupOptions: {
    //   external: ['naive-ui', 'echarts', 'vue', 'vue-router', 'pinia', 'axios'],
    //   output: {
    //     globals: {
    //       vue: 'Vue',
    //       'vue-router': 'VueRouter',
    //       pinia: 'Pinia',
    //       axios: 'axios',
    //       'naive-ui': 'naive',
    //       echarts: 'echarts',
    //     },
    //   },
    // },
  },

  // 预加载关键依赖
  optimizeDeps: {
    include: [
      'vue',
      'vue-router',
      'pinia',
      'axios',
      'naive-ui',
      'naive-ui/es/locales/date/zhCN',
      'naive-ui/es/locales/common/zhCN',
    ],
  },
})
