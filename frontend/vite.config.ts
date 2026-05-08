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
        target: 'http://localhost:8080',
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
    // 启用 CSS 压缩
    cssMinify: 'esbuild',
    // 启用模块预加载
    modulePreload: true,
    rollupOptions: {
      output: {
        // 更精细的手动分包
        manualChunks(id: string) {
          if (id.includes('node_modules')) {
            if (id.includes('naive-ui')) return 'vendor-naive'
            if (id.includes('vue')) return 'vendor-vue'
            if (id.includes('axios')) return 'vendor-axios'
            if (id.includes('pinia')) return 'vendor-vue'
            if (id.includes('vue-router')) return 'vendor-vue'
            return 'vendor-other'
          }
        },
        // 稳定 hash
        entryFileNames: 'assets/[name]-[hash:8].js',
        chunkFileNames: 'assets/[name]-[hash:8].js',
        assetFileNames: 'assets/[name]-[hash:8][extname]',
      },
    },
  },
})
