import { fileURLToPath, URL } from 'node:url'

import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import vueDevTools from 'vite-plugin-vue-devtools'
import tailwindcss from '@tailwindcss/vite'

// https://vite.dev/config/
export default defineConfig({
  plugins: [tailwindcss(), vue(), vueDevTools()],
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url)),
    },
  },
  server: {
    host: '127.0.0.1',
    port: 5174,
    proxy: {
      // 开发时把 /api 代理到本地 Rust 后端（BIND_ADDR 默认 0.0.0.0:8080）。
      '/api': {
        target: process.env.VITE_BACKEND_ORIGIN || 'http://127.0.0.1:8080',
        changeOrigin: true,
        // noVNC / 实时指标 WS 也会走 /api；没有 ws:true 时浏览器升级请求会一直卡住。
        ws: true,
      },
      // Swagger UI 与 OpenAPI JSON 也由后端提供。
      '/docs': {
        target: process.env.VITE_BACKEND_ORIGIN || 'http://127.0.0.1:8080',
        changeOrigin: true,
      },
      '/api-docs': {
        target: process.env.VITE_BACKEND_ORIGIN || 'http://127.0.0.1:8080',
        changeOrigin: true,
      },
      // 健康检查接口不在 /api/v1 下，单独代理。
      '/healthz': {
        target: process.env.VITE_BACKEND_ORIGIN || 'http://127.0.0.1:8080',
        changeOrigin: true,
      },
      '/readyz': {
        target: process.env.VITE_BACKEND_ORIGIN || 'http://127.0.0.1:8080',
        changeOrigin: true,
      },
    },
  },
})
