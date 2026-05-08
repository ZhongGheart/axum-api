/**
 * 用户管理页面
 *
 * 仅 admin 角色可访问。提供用户列表、搜索、新建、编辑、删除功能。
 */
<template>
  <div class="user-manage">
    <n-page-header title="用户管理">
      <template #extra>
        <PermissionButton permission="admin" type="primary" @click="openCreate">
          <template #icon><n-icon><AddIcon /></n-icon></template>
          新建用户
        </PermissionButton>
      </template>
    </n-page-header>

    <!-- 用户表格 -->
    <n-data-table
      :columns="columns"
      :data="userList"
      :loading="loading"
      :pagination="pagination"
      :bordered="true"
      class="user-table"
    />

    <!-- 新建/编辑对话框 -->
    <n-modal
      v-model:show="showModal"
      :title="isEditing ? '编辑用户' : '新建用户'"
      :mask-closable="false"
      preset="card"
      style="width: 520px"
    >
      <n-form ref="formRef" :model="formData" :rules="formRules" label-placement="left" label-width="80px">
        <n-form-item label="用户名" path="username">
          <n-input v-model:value="formData.username" :maxlength="50" />
        </n-form-item>
        <n-form-item label="邮箱" path="email">
          <n-input v-model:value="formData.email" :maxlength="255" />
        </n-form-item>
        <n-form-item v-if="!isEditing" label="密码" path="password">
          <n-input v-model:value="formData.password" type="password" />
        </n-form-item>
        <n-form-item label="角色" path="role">
          <n-select v-model:value="formData.role" :options="roleOptions" />
        </n-form-item>
        <n-form-item label="状态">
          <n-switch v-model:value="formData.is_active" />
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
import { ref, reactive, onMounted, h } from 'vue'
import { NButton, NTag, NSwitch } from 'naive-ui'
import { AddOutline as AddIcon } from '@vicons/ionicons5'
import type { DataTableColumn, FormInst, FormRules } from 'naive-ui'
import type { UserInfo } from '@/api/types/response'
import { userApi } from '@/api/user'
import { showSuccess, showConfirm } from '@/utils/message'
import PermissionButton from '@/components/common/PermissionButton.vue'

// ── 状态 ────────────────────────────────────────────────────────

const loading = ref(false)
const submitting = ref(false)
const showModal = ref(false)
const isEditing = ref(false)
const editingId = ref('')
const userList = ref<UserInfo[]>([])
const formRef = ref<FormInst | null>(null)

const pagination = reactive({
  page: 1,
  pageSize: 10,
  pageCount: 1,
  showSizePicker: true,
  pageSizes: [10, 20, 50],
  onChange: (page: number) => {
    pagination.page = page
    fetchUsers()
  },
  onUpdatePageSize: (size: number) => {
    pagination.pageSize = size
    pagination.page = 1
    fetchUsers()
  },
})

const roleOptions = [
  { label: '管理员', value: 'admin' },
  { label: '普通用户', value: 'user' },
]

interface UserForm {
  username: string
  email: string
  password: string
  role: string
  is_active: boolean
}

const formData = reactive<UserForm>({
  username: '',
  email: '',
  password: '',
  role: 'user',
  is_active: true,
})

const formRules: FormRules = {
  username: [
    { required: true, message: '请输入用户名' },
    { min: 3, message: '至少 3 个字符' },
    { max: 50, message: '不超过 50 个字符' },
  ],
  email: [
    { required: true, message: '请输入邮箱' },
    { type: 'email', message: '邮箱格式不正确' },
  ],
  password: [
    { min: 6, message: '密码至少 6 个字符', trigger: 'blur' },
  ],
  role: [{ required: true, message: '请选择角色' }],
}

// ── 表格列定义 ──────────────────────────────────────────────────

const columns: DataTableColumn[] = [
  { title: '用户名', key: 'username', width: 150 },
  { title: '邮箱', key: 'email', width: 200 },
  {
    title: '角色',
    key: 'role',
    width: 100,
    render(row: Record<string, unknown>) {
      const r = row as unknown as UserInfo
      return h(NTag, { type: r.role === 'admin' ? 'warning' : 'info', size: 'small' }, () => r.role)
    },
  },
  {
    title: '状态',
    key: 'is_active',
    width: 80,
    render(row: Record<string, unknown>) {
      return h(NSwitch, { value: row.is_active as boolean, disabled: true })
    },
  },
  { title: '创建时间', key: 'created_at', width: 180 },
  {
    title: '操作',
    key: 'actions',
    width: 160,
    render(row: Record<string, unknown>) {
      const r = row as unknown as UserInfo
      return h('div', { style: 'display:flex;gap:8px' }, [
        h(PermissionButton, { permission: 'admin', size: 'small', onClick: () => openEdit(r) }, () => '编辑'),
        h(PermissionButton, { permission: 'admin', size: 'small', type: 'error', onClick: () => handleDelete(r.id) }, () => '删除'),
      ])
    },
  },
]

// ── 数据加载 ────────────────────────────────────────────────────

async function fetchUsers() {
  loading.value = true
  try {
    const res = await userApi.list({ page: pagination.page, page_size: pagination.pageSize })
    const data = res as unknown as { items: UserInfo[]; total: number; page: number; page_size: number; total_pages: number }
    userList.value = data.items
    pagination.pageCount = data.total_pages
    pagination.page = data.page
  } catch {
    // handled by interceptor
  } finally {
    loading.value = false
  }
}

// ── 新建/编辑 ───────────────────────────────────────────────────

function openCreate() {
  isEditing.value = false
  editingId.value = ''
  formData.username = ''
  formData.email = ''
  formData.password = ''
  formData.role = 'user'
  formData.is_active = true
  showModal.value = true
}

function openEdit(user: UserInfo) {
  isEditing.value = true
  editingId.value = user.id
  formData.username = user.username
  formData.email = user.email
  formData.password = ''
  formData.role = user.role
  formData.is_active = user.is_active
  showModal.value = true
}

async function handleSubmit() {
  try {
    await formRef.value?.validate()
    submitting.value = true

    if (isEditing.value) {
      await userApi.update(editingId.value, {
        username: formData.username,
        email: formData.email,
        role: formData.role,
        is_active: formData.is_active,
      })
      showSuccess('用户更新成功')
    } else {
      await userApi.create({
        username: formData.username,
        email: formData.email,
        password: formData.password || undefined,
        role: formData.role,
      })
      showSuccess('用户创建成功')
    }

    showModal.value = false
    fetchUsers()
  } catch {
    // handled by interceptor
  } finally {
    submitting.value = false
  }
}

async function handleDelete(id: string) {
  const confirmed = await showConfirm({ content: '确定要删除该用户吗？' })
  if (!confirmed) return

  try {
    await userApi.delete(id)
    showSuccess('用户已删除')
    fetchUsers()
  } catch {
    // handled
  }
}

// ── 初始化 ──────────────────────────────────────────────────────

onMounted(() => {
  fetchUsers()
})
</script>

<style scoped>
.user-manage {
  padding: 24px;
}
.user-table {
  margin-top: 16px;
}
</style>
