<template>
  <img
    v-if="loaded"
    v-bind="$attrs"
    :src="src"
    :style="{ opacity: 1, transition: 'opacity 0.3s' }"
    @error="onError"
  />
  <div
    v-else
    class="lazy-image-placeholder"
    :style="{ width: width || '100%', height: height || '100%' }"
  >
    <slot name="placeholder">
      <div class="lazy-image-loading">
        <n-spin size="small" />
      </div>
    </slot>
  </div>
  <img
    v-show="false"
    :src="src"
    @load="onRealLoad"
    @error="onError"
  />
</template>

<script setup lang="ts">
/**
 * 图片懒加载组件
 *
 * 使用 IntersectionObserver 延迟加载图片，
 * 只有当图片进入视口时才发起下载。
 *
 * 使用方式：
 *   <LazyImage src="https://example.com/large.jpg" width="400" height="300" />
 */
import { ref, onMounted, onBeforeUnmount } from 'vue'
import { NSpin } from 'naive-ui'

const props = withDefaults(
  defineProps<{
    src: string
    width?: string
    height?: string
    /** 根边距（提前加载） */
    rootMargin?: string
    threshold?: number
  }>(),
  { rootMargin: '200px', threshold: 0.01 },
)

const emit = defineEmits<{
  load: []
  error: [err: Error]
}>()

const loaded = ref(false)
const observer = ref<IntersectionObserver | null>(null)
const target = ref<HTMLElement | null>(null)

onMounted(() => {
  if ('IntersectionObserver' in window) {
    observer.value = new IntersectionObserver(
      ([entry]) => {
        if (entry.isIntersecting) {
          // 触发加载：设置 hidden img 的 src 来真正加载
          loaded.value = true
          observer.value?.disconnect()
        }
      },
      { rootMargin: props.rootMargin, threshold: props.threshold },
    )
    // 观察占位元素
    if (target.value) observer.value.observe(target.value)
  } else {
    // 不支持 IntersectionObserver 时直接加载
    loaded.value = true
  }
})

onBeforeUnmount(() => {
  observer.value?.disconnect()
})

function onRealLoad() {
  emit('load')
}

function onError() {
  emit('error', new Error(`图片加载失败: ${props.src}`))
}
</script>

<style scoped>
.lazy-image-placeholder {
  display: flex;
  align-items: center;
  justify-content: center;
  background: #f5f5f5;
  border-radius: 4px;
  overflow: hidden;
}

.lazy-image-loading {
  display: flex;
  align-items: center;
  justify-content: center;
}
</style>
