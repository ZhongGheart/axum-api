/**
 * 角色管理页面
 *
 * 角色增删改 + 菜单/权限码授权。后端 `PUT /admin/roles/:id/menus` 是
 * **全量覆盖**语义，因此授权弹窗用全量菜单树渲染，把
 * `GET /admin/menus?role_id=` 的结果**仅**用作默认勾选值，
 * 提交时给完整勾选集合——既不静默丢授权，也不因树结构失真而静默扩权。
 */
<template>
  <div class="page-container">
    <n-page-header title="角色管理">
      <template #extra>
        <PermissionButton :permission="PERM.ROLE_CREATE" type="primary" @click="openCreate">
          <template #icon><n-icon><AddIcon /></n-icon></template>
          新建角色
        </PermissionButton>
      </template>
    </n-page-header>

    <n-data-table
      :columns="columns"
      :data="roleList"
      :loading="loading"
      :bordered="true"
      class="role-table"
    />

    <!-- 新建/编辑角色 -->
    <n-modal
      v-model:show="showModal"
      :title="isEditing ? '编辑角色' : '新建角色'"
      :mask-closable="false"
      preset="card"
      style="width: 480px"
    >
      <n-form ref="formRef" :model="formData" :rules="formRules" label-placement="left" label-width="80px">
        <n-form-item label="角色名称" path="name">
          <n-input v-model:value="formData.name" :maxlength="50" placeholder="如 auditor" />
        </n-form-item>
        <n-form-item label="描述" path="description">
          <n-input v-model:value="formData.description" type="textarea" :maxlength="200" :rows="2" />
        </n-form-item>
      </n-form>
      <template #footer>
        <n-space justify="end">
          <n-button @click="showModal = false">取消</n-button>
          <n-button type="primary" :loading="submitting" @click="handleSubmit">保存</n-button>
        </n-space>
      </template>
    </n-modal>

    <!-- 菜单 / 权限码授权 -->
    <n-modal
      v-model:show="showGrantModal"
      :title="`授权菜单与权限码 — ${grantRole?.name ?? ''}`"
      preset="card"
      style="width: 620px"
    >
      <n-alert type="info" :bordered="false" class="grant-hint">
        保存为全量覆盖：未勾选的菜单与权限码会被撤销。「按钮」节点即接口权限码，
        授予后才能通过后端权限码校验。当前已选 {{ grantedCount }} 项。
      </n-alert>
      <n-spin :show="grantLoading">
        <n-tree
          v-if="grantTreeData.length"
          :data="grantTreeData"
          :default-expand-all="true"
          :render-label="renderGrantLabel"
          :checked-keys="grantCheckedKeys"
          block-line
          checkable
          cascade
          @update:checked-keys="onCheckedKeysChange"
        />
        <n-empty v-else description="暂无菜单数据" />
      </n-spin>
      <template #footer>
        <n-space justify="end">
          <n-button @click="showGrantModal = false">取消</n-button>
          <n-button type="primary" :loading="submitting" @click="handleGrantSubmit">保存授权</n-button>
        </n-space>
      </template>
    </n-modal>
  </div>
</template>

<script setup lang="ts">
import { ref, h, onMounted, computed } from 'vue'
import { NTag, NSpace, NIcon, NTooltip } from 'naive-ui'
import { AddOutline as AddIcon, ShieldCheckmarkOutline as GrantIcon } from '@vicons/ionicons5'
import type { DataTableColumn, FormInst, FormRules, TreeOption } from 'naive-ui'
import type { RoleItem, RoleReq } from '@/api/role'
import { roleApi } from '@/api/role'
import { menuApi } from '@/api/menu'
import type { MenuNode } from '@/api/menu'
import { buildGrantTreeOptions, authorizedMenuIds, menuTypeLabel, filterKnownMenuIds } from '@/utils/menu'
import { isBuiltinRole } from '@/constants/builtin'
import { PERM } from '@/constants/permission'
import { showConfirm, showSuccess } from '@/utils/message'
import PermissionButton from '@/components/common/PermissionButton.vue'

// ── 状态 ────────────────────────────────────────────────────────

const loading = ref(false)
const submitting = ref(false)
const roleList = ref<RoleItem[]>([])

const showModal = ref(false)
const isEditing = ref(false)
const editingId = ref('')
const formRef = ref<FormInst | null>(null)

const showGrantModal = ref(false)
const grantLoading = ref(false)
const grantRole = ref<RoleItem | null>(null)
const grantTreeData = ref<TreeOption[]>([])
const grantCheckedKeys = ref<Array<string | number>>([])

const grantedCount = computed(() => grantCheckedKeys.value.length)

interface RoleForm {
  name: string
  description: string
}

const formData = ref<RoleForm>({ name: '', description: '' })

const formRules: FormRules = {
  name: [
    { required: true, message: '请输入角色名称' },
    { max: 50, message: '不超过 50 个字符' },
  ],
}

// ── 表格列 ──────────────────────────────────────────────────────

const columns: DataTableColumn[] = [
  {
    title: '角色名称',
    key: 'name',
    width: 180,
    render(row: Record<string, unknown>) {
      const name = String(row.name)
      const label = h('span', name)
      if (!isBuiltinRole(name)) return label
      return h(NSpace, { size: 'small', align: 'center' }, {
        default: () => [
          label,
          h(NTag, { size: 'small', type: 'warning', bordered: false }, () => '内置'),
          h(
            NTooltip,
            { trigger: 'hover' },
            {
              trigger: () => h(NIcon, { style: 'cursor:help' }, () => h(GrantIcon)),
              default: () => '内置角色不可删除：角色种子仅在角色表为空时写入，删除后不会重建',
            },
          ),
        ],
      })
    },
  },
  { title: '描述', key: 'description', ellipsis: { tooltip: true } },
  {
    title: '用户数',
    key: 'user_count',
    width: 80,
    render(row: Record<string, unknown>) {
      return h(NTag, { type: 'info', size: 'small' }, () => String(row.user_count))
    },
  },
  {
    title: '创建时间',
    key: 'created_at',
    width: 180,
    render(row: Record<string, unknown>) {
      return h('span', String(row.created_at ?? '').replace('T', ' ').slice(0, 19))
    },
  },
  {
    title: '操作',
    key: 'actions',
    width: 200,
    render(row: Record<string, unknown>) {
      const role = row as unknown as RoleItem
      const actions = [
        h(PermissionButton, { permission: PERM.ROLE_UPDATE, size: 'small', onClick: () => openEdit(role) }, () => '编辑'),
        h(PermissionButton, { permission: PERM.MENU_GRANT, size: 'small', onClick: () => openGrant(role) }, () => '授权'),
        // 内置角色不渲染删除按钮：后端必然返回 400
        isBuiltinRole(role.name)
          ? false
          : h(
              PermissionButton,
              { permission: PERM.ROLE_DELETE, size: 'small', type: 'error', onClick: () => handleDelete(role) },
              () => '删除',
            ),
      ].filter(Boolean)
      return h('div', { style: 'display:flex;gap:8px' }, actions)
    },
  },
]

// ── 数据加载 ────────────────────────────────────────────────────

async function fetchRoles() {
  loading.value = true
  try {
    const res = await roleApi.list()
    roleList.value = res as unknown as RoleItem[]
  } catch {
    // handled by interceptor
  } finally {
    loading.value = false
  }
}

// ── 新建 / 编辑 ─────────────────────────────────────────────────

function openCreate() {
  isEditing.value = false
  editingId.value = ''
  formData.value = { name: '', description: '' }
  showModal.value = true
}

function openEdit(role: RoleItem) {
  isEditing.value = true
  editingId.value = role.id
  formData.value = { name: role.name, description: role.description ?? '' }
  showModal.value = true
}

async function handleSubmit() {
  try {
    await formRef.value?.validate()
    submitting.value = true
    const payload: RoleReq = {
      name: formData.value.name.trim(),
      description: formData.value.description.trim() || undefined,
    }
    if (isEditing.value) {
      await roleApi.update(editingId.value, payload)
      showSuccess('角色更新成功')
    } else {
      await roleApi.create(payload)
      showSuccess('角色创建成功')
    }
    showModal.value = false
    fetchRoles()
  } catch {
    // handled by interceptor
  } finally {
    submitting.value = false
  }
}

// ── 授权 ────────────────────────────────────────────────────────

async function openGrant(role: RoleItem) {
  grantRole.value = role
  grantCheckedKeys.value = []
  grantTreeData.value = []
  showGrantModal.value = true
  grantLoading.value = true
  try {
    const [all, granted] = await Promise.all([menuApi.list(), menuApi.listByRole(role.id)])
    // 全量树负责渲染（父子关系真实），授权结果只作为默认勾选值
    grantTreeData.value = buildGrantTreeOptions(all as unknown as MenuNode[])
    grantCheckedKeys.value = authorizedMenuIds(granted as unknown as MenuNode[])
  } catch {
    // handled by interceptor
  } finally {
    grantLoading.value = false
  }
}

function onCheckedKeysChange(keys: Array<string | number>) {
  grantCheckedKeys.value = keys
}

async function handleGrantSubmit() {
  const role = grantRole.value
  if (!role) return
  try {
    submitting.value = true
    const menuIds = filterKnownMenuIds(grantCheckedKeys.value, grantTreeData.value)
    await roleApi.assignMenus(role.id, menuIds)
    showSuccess(`已保存「${role.name}」的授权`)
    showGrantModal.value = false
  } catch {
    // handled by interceptor
  } finally {
    submitting.value = false
  }
}

function renderGrantLabel({ option }: { option: TreeOption }) {
  const meta = option as TreeOption & { menuType?: string; permission?: string | null }
  const type = meta.menuType ?? 'menu'
  const tagType = type === 'button' ? 'info' : type === 'directory' ? 'warning' : 'success'
  return h('div', { style: 'display:flex;align-items:center;gap:8px;padding:4px 0' }, [
    h('span', option.label as string),
    h(NTag, { size: 'small', type: tagType, bordered: false }, () => menuTypeLabel(type)),
    meta.permission ? h('code', { style: 'font-size:12px;opacity:0.7' }, meta.permission) : null,
  ])
}

// ── 删除 ────────────────────────────────────────────────────────

async function handleDelete(role: RoleItem) {
  const content =
    role.user_count > 0
      ? `确定删除角色「${role.name}」？该角色当前有 ${role.user_count} 个用户，需先调整这些用户的角色。`
      : `确定删除角色「${role.name}」？其菜单与权限码授权会一并清除。`
  const confirmed = await showConfirm({ content })
  if (!confirmed) return

  try {
    await roleApi.delete(role.id)
    showSuccess('角色已删除')
    fetchRoles()
  } catch {
    // handled by interceptor
  }
}

onMounted(() => {
  fetchRoles()
})
</script>

<style scoped>
.role-table {
  margin-top: 16px;
}

.grant-hint {
  margin-bottom: 12px;
}
</style>
