/**
 * 用户管理页面
 *
 * 仅 admin 角色可访问。使用 BaseTable 通用表格组件。
 */
<template>
  <div class="page-container">
    <n-page-header title="用户管理">
      <template #extra>
        <PermissionButton :permission="PERM.USER_CREATE" type="primary" @click="openCreate">
          <template #icon><n-icon><AddIcon /></n-icon></template>
          新建用户
        </PermissionButton>
      </template>
    </n-page-header>

    <!-- 搜索栏 -->
    <SearchForm @search="onSearch" @clear="onSearchClear" />

    <!-- 数据表格 -->
    <BaseTable
      :columns="columns"
      :data="userList"
      :loading="loading"
      :total="total"
      v-model:page="page"
      v-model:page-size="pageSize"
      @update:page="fetchUsers"
      @update:page-size="onPageSizeChange"
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
          <n-select
            v-model:value="formData.role"
            :options="roleOptions"
            :disabled="!!rolesUnavailable"
            :placeholder="rolesUnavailable ? '角色列表不可用' : '请选择角色'"
          />
          <div v-if="rolesUnavailable" style="color: #d03050; font-size: 12px; margin-top: 4px">
            {{ rolesUnavailable }}
          </div>
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
import { ref, h, onMounted } from 'vue'
import { NTag, NSwitch } from 'naive-ui'
import { AddOutline as AddIcon } from '@vicons/ionicons5'
import type { DataTableColumn, FormInst, FormRules } from 'naive-ui'
import type { UserInfo } from '@/api/types/response'
import { userApi } from '@/api/user'
import { showSuccess, showConfirm } from '@/utils/message'
import BaseTable from '@/components/common/BaseTable.vue'
import SearchForm from '@/components/common/SearchForm.vue'
import PermissionButton from '@/components/common/PermissionButton.vue'
import { PERM } from '@/constants/permission'
import { ADMIN_ROLE_NAME } from '@/constants/builtin'
import { roleApi, type RoleItem } from '@/api/role'
import type { RoleListItem, RoleSelectOption } from '@/utils/role'
import {
  buildRoleSelectOptions,
  currentRoleName,
  DEFAULT_ROLE_NAME,
  pickDefaultRole,
} from '@/utils/role'

// ── 状态 ────────────────────────────────────────────────────────

const loading = ref(false)
const submitting = ref(false)
const showModal = ref(false)
const isEditing = ref(false)
const editingId = ref('')
const userList = ref<Record<string, unknown>[]>([])
const total = ref(0)
const page = ref(1)
const pageSize = ref(10)
const formRef = ref<FormInst | null>(null)

const roleOptions = ref<RoleSelectOption[]>([])
const availableRoles = ref<RoleListItem[]>([])
const rolesUnavailable = ref<string | null>(null)

/**
 * 按需加载角色列表，并缓存结果
 *
 * 刻意懒加载：`GET /admin/roles` 需要 `system:role:list`，
 * 只浏览用户列表、没有建/改用户权限的账号根本不该被要求具备这个权限。
 * 失败不重试——多半是权限不足，重试只会刷一遍错误提示。
 */
async function ensureRolesLoaded() {
  if (availableRoles.value.length > 0 || rolesUnavailable.value) return
  try {
    // 响应拦截器已拆出 data.data，但类型上仍是 AxiosResponse——沿用仓库既有写法
    availableRoles.value = (await roleApi.list()) as unknown as RoleItem[]
  } catch {
    rolesUnavailable.value = '无法读取角色列表，请检查是否具备角色查看权限'
  }
}

interface UserForm {
  username: string
  email: string
  password: string
  role: string
  is_active: boolean
}

const formData = ref<UserForm>({
  username: '',
  email: '',
  password: '',
  role: DEFAULT_ROLE_NAME,
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
  password: [{ min: 6, message: '密码至少 6 个字符', trigger: 'blur' }],
  role: [{ required: true, message: '请选择角色' }],
}

// ── 表格列 ──────────────────────────────────────────────────────

const columns: DataTableColumn[] = [
  { title: '用户名', key: 'username', width: 150 },
  { title: '邮箱', key: 'email', width: 200 },
  {
    title: '角色',
    key: 'role',
    width: 100,
    render(row: Record<string, unknown>) {
      const r = row as unknown as UserInfo
      // 显示真实角色集合：r.role 是两值枚举，自定义角色会被塌缩成 "user"
      const name = currentRoleName(r)
      return h(NTag, { type: name === ADMIN_ROLE_NAME ? 'warning' : 'info', size: 'small' }, () => name)
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
        h(PermissionButton, { permission: PERM.USER_UPDATE, size: 'small', onClick: () => openEdit(r) }, () => '编辑'),
        h(PermissionButton, { permission: PERM.USER_DELETE, size: 'small', type: 'error', onClick: () => handleDelete(r.id) }, () => '删除'),
      ])
    },
  },
]

// ── 数据加载 ────────────────────────────────────────────────────

async function fetchUsers() {
  loading.value = true
  try {
    const res = await userApi.list({ page: page.value, page_size: pageSize.value })
    const data = res as unknown as {
      items: UserInfo[]
      total: number
      page: number
      page_size: number
    }
    userList.value = data.items as unknown as Record<string, unknown>[]
    total.value = data.total
  } catch {
    // handled by interceptor
  } finally {
    loading.value = false
  }
}

function onPageSizeChange(size: number) {
  pageSize.value = size
  page.value = 1
  fetchUsers()
}

function onSearch(_keyword: string) {
  page.value = 1
  // 搜索逻辑由具体业务实现
  fetchUsers()
}

function onSearchClear() {
  page.value = 1
  fetchUsers()
}

// ── 新建/编辑 ───────────────────────────────────────────────────

async function openCreate() {
  await ensureRolesLoaded()
  isEditing.value = false
  editingId.value = ''
  roleOptions.value = buildRoleSelectOptions(availableRoles.value)
  formData.value = {
    username: '',
    email: '',
    password: '',
    role: pickDefaultRole(availableRoles.value),
    is_active: true,
  }
  showModal.value = true
}

async function openEdit(user: UserInfo) {
  await ensureRolesLoaded()
  isEditing.value = true
  editingId.value = user.id
  // 必须取真实角色集合，不能用 user.role：后者是非 admin 一律塌缩成 "user"
  // 的展示枚举，回填它会把自定义角色静默改掉
  const current = currentRoleName(user)
  // 带上当前角色：它可能不在列表里（历史遗留的非归一化角色名）
  roleOptions.value = buildRoleSelectOptions(availableRoles.value, current)
  formData.value = {
    username: user.username,
    email: user.email,
    password: '',
    role: current,
    is_active: user.is_active,
  }
  showModal.value = true
}

async function handleSubmit() {
  try {
    await formRef.value?.validate()
    submitting.value = true

    if (isEditing.value) {
      await userApi.update(editingId.value, {
        username: formData.value.username,
        email: formData.value.email,
        role: formData.value.role,
        is_active: formData.value.is_active,
      })
      showSuccess('用户更新成功')
    } else {
      await userApi.create({
        username: formData.value.username,
        email: formData.value.email,
        password: formData.value.password || undefined,
        role: formData.value.role,
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

onMounted(() => {
  fetchUsers()
})
</script>
