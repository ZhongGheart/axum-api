<template>
  <div class="page-container">
    <n-page-header title="菜单管理">
      <template #extra>
        <n-button type="primary" @click="openCreate(null)">新增根菜单</n-button>
      </template>
    </n-page-header>

    <n-card>
      <n-tree
        :data="treeData"
        :default-expand-all="true"
        :render-label="renderLabel"
        block-line
        checkable
        cascade
      />
    </n-card>

    <!-- 新增/编辑弹窗 -->
    <n-modal v-model:show="showModal" :title="isEditing ? '编辑菜单' : '新增菜单'" preset="card" style="width:560px">
      <n-form ref="formRef" :model="formData" :rules="rules" label-placement="left" label-width="80px">
        <n-form-item label="菜单名称" path="name">
          <n-input v-model:value="formData.name" />
        </n-form-item>
        <n-form-item label="类型" path="type">
          <n-select v-model:value="formData.type" :options="typeOptions" />
        </n-form-item>
        <n-form-item label="路由路径" path="path">
          <n-input v-model:value="formData.path" placeholder="/system/user" />
        </n-form-item>
        <n-form-item label="图标" path="icon">
          <n-input v-model:value="formData.icon" placeholder="SettingsOutline" />
        </n-form-item>
        <n-form-item label="排序" path="sort_order">
          <n-input-number v-model:value="formData.sort_order" :min="0" />
        </n-form-item>
        <n-form-item label="权限标识" path="permission">
          <n-input v-model:value="formData.permission" placeholder="system:user:edit" />
        </n-form-item>
        <n-form-item label="是否可见">
          <n-switch v-model:value="formData.is_visible" />
        </n-form-item>
      </n-form>
      <template #footer>
        <n-space justify="end">
          <n-button @click="showModal = false">取消</n-button>
          <n-button type="primary" :loading="submitting" @click="handleSubmit">保存</n-button>
        </n-space>
      </template>
    </n-modal>
  </div>
</template>

<script setup lang="ts">
import { ref, h, onMounted } from 'vue'
import { NButton, NSpace, NIcon } from 'naive-ui'
import { AddOutline as AddIcon, CreateOutline as EditIcon, TrashOutline as DelIcon } from '@vicons/ionicons5'
import type { FormInst, FormRules, TreeOption } from 'naive-ui'
import { menuApi } from '@/api/menu'
import type { MenuNode, CreateMenuReq } from '@/api/menu'
import { showConfirm, showSuccess } from '@/utils/message'

const formRef = ref<FormInst | null>(null)
const showModal = ref(false)
const isEditing = ref(false)
const editingId = ref('')
const parentId = ref<string | null>(null)
const submitting = ref(false)
const treeData = ref<TreeOption[]>([])

const typeOptions = [
  { label: '目录', value: 'directory' },
  { label: '菜单', value: 'menu' },
  { label: '按钮', value: 'button' },
]

interface MenuForm {
  name: string
  type: 'menu' | 'button' | 'directory'
  path: string
  icon: string
  sort_order: number
  permission: string
  is_visible: boolean
}

const formData = ref<MenuForm>({
  name: '', type: 'menu', path: '', icon: '', sort_order: 0, permission: '', is_visible: true,
})

const rules: FormRules = {
  name: [{ required: true, message: '请输入菜单名称' }],
  type: [{ required: true, message: '请选择类型' }],
}

async function fetchTree() {
  const res = await menuApi.list()
  treeData.value = buildTreeOptions(res as unknown as MenuNode[])
}

function buildTreeOptions(nodes: MenuNode[]): TreeOption[] {
  return nodes.map((n) => ({
    key: n.id,
    label: n.name,
    children: n.children?.length ? buildTreeOptions(n.children) : undefined,
    isLeaf: !n.children?.length,
  }))
}

function renderLabel({ option }: { option: TreeOption }) {
  return h('div', { style: 'display:flex;align-items:center;gap:8px;padding:4px 0' }, [
    h('span', option.label as string),
    h(NSpace, { size: 'small' }, {
      default: () => [
        h(NButton, { size: 'tiny', quaternary: true, onClick: () => openCreate(option.key as string) }, { default: () => h(NIcon, null, () => h(AddIcon)) }),
        h(NButton, { size: 'tiny', quaternary: true, onClick: () => openEdit(option.key as string) }, { default: () => h(NIcon, null, () => h(EditIcon)) }),
        h(NButton, { size: 'tiny', quaternary: true, type: 'error', onClick: () => handleDelete(option.key as string) }, { default: () => h(NIcon, null, () => h(DelIcon)) }),
      ],
    }),
  ])
}

async function openCreate(pid: string | null) {
  isEditing.value = false
  editingId.value = ''
  parentId.value = pid
  formData.value = { name: '', type: 'menu', path: '', icon: '', sort_order: 0, permission: '', is_visible: true }
  showModal.value = true
}

async function openEdit(id: string) {
  isEditing.value = true
  editingId.value = id
  showModal.value = true
}

async function handleSubmit() {
  try {
    await formRef.value?.validate()
    submitting.value = true
    const data: CreateMenuReq = { ...formData.value, parent_id: parentId.value || undefined }
    if (isEditing.value) {
      await menuApi.update(editingId.value, data)
      showSuccess('更新成功')
    } else {
      await menuApi.create(data)
      showSuccess('创建成功')
    }
    showModal.value = false
    fetchTree()
  } catch { /* handled */ } finally { submitting.value = false }
}

async function handleDelete(id: string) {
  const ok = await showConfirm({ content: '确定删除该菜单及其子菜单？' })
  if (!ok) return
  await menuApi.delete(id)
  showSuccess('删除成功')
  fetchTree()
}

onMounted(fetchTree)
</script>
