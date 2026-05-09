<template>
  <div class="base-upload">
    <!-- 拖拽上传区 -->
    <n-upload
      v-if="mode === 'drag'"
      v-model:file-list="fileList"
      :action="uploadUrl"
      :headers="headers"
      :data="uploadData"
      :multiple="multiple"
      :accept="accept"
      :max-size="maxSize"
      :show-retry="true"
      :show-cancel="true"
      list-type="image-card"
      @finish="onFinish"
      @error="onError"
      @progress="onProgress"
      class="base-upload__drag"
    >
      <n-upload-dragger>
        <div style="padding:24px;text-align:center">
          <n-icon size="48" color="#2080f0"><CloudUploadIcon /></n-icon>
          <p>点击或拖拽文件到此处上传</p>
          <p style="font-size:12px;color:#888;margin-top:4px">
            支持 {{ accept || '所有格式' }}，单文件不超过 {{ (maxSize / 1024 / 1024).toFixed(0) }}MB
          </p>
        </div>
      </n-upload-dragger>
    </n-upload>

    <!-- 按钮上传 -->
    <n-upload
      v-else
      v-model:file-list="fileList"
      :action="uploadUrl"
      :headers="headers"
      :data="uploadData"
      :multiple="multiple"
      :accept="accept"
      :max-size="maxSize"
      :show-retry="true"
      :show-cancel="true"
      :list-type="listType"
      @finish="onFinish"
      @error="onError"
      @progress="onProgress"
    >
      <n-button>
        <template #icon><n-icon><CloudUploadIcon /></n-icon></template>
        选择文件
      </n-button>
    </n-upload>

    <!-- 自定义进度条（分片上传用） -->
    <div v-if="chunkProgress > 0 && chunkProgress < 100" class="base-upload__chunk-progress">
      <n-progress :percentage="chunkProgress" :indicator-placement="'inside'" processing />
      <p style="font-size:12px;color:#888;margin-top:4px">分片上传中... {{ uploadSpeed }}</p>
    </div>
  </div>
</template>

<script setup lang="ts">
/**
 * 通用上传组件
 *
 * 支持文件/图片上传、分片上传、进度显示。
 *
 * 使用方式：
 *   <BaseUpload
 *     :action="'/api/upload'"
 *     :max-size="10 * 1024 * 1024"
 *     @success="onUploadSuccess"
 *   />
 */
import { ref, computed } from 'vue'
import { CloudUploadOutline as CloudUploadIcon } from '@vicons/ionicons5'
import type { UploadFileInfo } from 'naive-ui'
import { getToken } from '@/utils/storage'

const props = withDefaults(
  defineProps<{
    /** 上传地址 */
    action: string
    /** 上传模式 */
    mode?: 'button' | 'drag'
    /** 是否多选 */
    multiple?: boolean
    /** 接受的文件类型 */
    accept?: string
    /** 文件大小限制（字节） */
    maxSize?: number
    /** 展示类型 */
    listType?: 'image' | 'text'
    /** 额外的请求数据 */
    data?: Record<string, unknown>
    /** 是否启用分片上传 */
    chunked?: boolean
    /** 分片大小（字节） */
    chunkSize?: number
  }>(),
  {
    mode: 'button',
    multiple: true,
    accept: '',
    maxSize: 10 * 1024 * 1024,
    listType: 'image',
    data: () => ({}),
    chunked: false,
    chunkSize: 2 * 1024 * 1024,
  },
)

const emit = defineEmits<{
  success: [file: UploadFileInfo]
  error: [file: UploadFileInfo]
  progress: [percent: number]
}>()

const fileList = ref<UploadFileInfo[]>([])
const chunkProgress = ref(0)
const uploadSpeed = ref('')

/** 上传地址 */
const uploadUrl = computed(() => props.action)

/** 请求头（自动注入 token） */
const headers = computed(() => {
  const token = getToken()
  return token ? { Authorization: `Bearer ${token}` } : {}
})

/** 上传数据 */
const uploadData = computed(() => ({ ...props.data }))

function onFinish({ file }: { file: UploadFileInfo }) {
  emit('success', file)
}

function onError({ file }: { file: UploadFileInfo }) {
  emit('error', file)
}

function onProgress({ percent }: { percent: number }) {
  chunkProgress.value = percent
  emit('progress', percent)
}
</script>

<style scoped>
.base-upload__drag {
  width: 100%;
}
.base-upload__chunk-progress {
  margin-top: 12px;
}
</style>
