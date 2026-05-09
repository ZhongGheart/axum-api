<template>
  <n-modal
    v-model:show="visible"
    :title="title"
    :preset="preset"
    :mask-closable="maskClosable"
    :close-on-esc="closeOnEsc"
    :segmented="segmented"
    :transform-origin="transformOrigin"
    :style="modalStyle"
    :class="{ 'base-modal--draggable': draggable }"
    @update:show="onClose"
  >
    <!-- 拖拽手柄 -->
    <template v-if="draggable" #header>
      <div class="base-modal__drag-handle" @mousedown="onDragStart">
        <n-icon><GridIcon /></n-icon>
        <span>{{ title }}</span>
      </div>
    </template>

    <!-- 默认内容插槽 -->
    <slot />

    <!-- 底部操作栏 -->
    <template v-if="$slots.footer || showOk" #footer>
      <slot name="footer">
        <n-space justify="end">
          <n-button @click="onClose">取消</n-button>
          <n-button type="primary" :loading="confirmLoading" @click="$emit('ok')">
            {{ okText }}
          </n-button>
        </n-space>
      </slot>
    </template>
  </n-modal>
</template>

<script setup lang="ts">
/**
 * 通用弹窗组件
 *
 * 支持拖拽、自适应高度、嵌套。
 *
 * 使用方式：
 *   <BaseModal v-model:show="visible" title="编辑用户" @ok="handleSave">
 *     <n-form>...</n-form>
 *   </BaseModal>
 */
import { ref, computed } from 'vue'
import { GridOutline as GridIcon } from '@vicons/ionicons5'

const props = withDefaults(
  defineProps<{
    show: boolean
    title?: string
    preset?: 'card' | 'dialog'
    width?: number | string
    maskClosable?: boolean
    closeOnEsc?: boolean
    draggable?: boolean
    showOk?: boolean
    okText?: string
    confirmLoading?: boolean
    segmented?: boolean | { header?: boolean; footer?: boolean }
    transformOrigin?: 'center' | 'mouse'
  }>(),
  {
    title: '弹窗',
    preset: 'card',
    width: 520,
    maskClosable: false,
    closeOnEsc: true,
    draggable: false,
    showOk: true,
    okText: '确定',
    confirmLoading: false,
    segmented: false,
    transformOrigin: 'mouse',
  },
)

const emit = defineEmits<{
  'update:show': [value: boolean]
  ok: []
}>()

const visible = computed({
  get: () => props.show,
  set: (val: boolean) => emit('update:show', val),
})

const modalStyle = computed(() => ({
  width: typeof props.width === 'number' ? `${props.width}px` : props.width,
}))

function onClose() {
  emit('update:show', false)
}

// ── 拖拽逻辑 ──────────────────────────────────────────────────

let startX = 0, startY = 0, offsetX = 0, offsetY = 0

function onDragStart(e: MouseEvent) {
  const target = (e.currentTarget as HTMLElement)?.closest('.n-modal') as HTMLElement
  if (!target) return
  const rect = target.getBoundingClientRect()
  offsetX = e.clientX - rect.left
  offsetY = e.clientY - rect.top

  function onMove(ev: MouseEvent) {
    target.style.left = `${ev.clientX - offsetX}px`
    target.style.top = `${ev.clientY - offsetY}px`
    target.style.transform = 'none'
    target.style.margin = '0'
  }
  function onUp() {
    document.removeEventListener('mousemove', onMove)
    document.removeEventListener('mouseup', onUp)
  }
  document.addEventListener('mousemove', onMove)
  document.addEventListener('mouseup', onUp)
}
</script>

<style scoped>
.base-modal__drag-handle {
  display: flex;
  align-items: center;
  gap: 8px;
  cursor: grab;
  user-select: none;
}
.base-modal__drag-handle:active {
  cursor: grabbing;
}
</style>
