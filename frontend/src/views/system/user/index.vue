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
    <SearchForm @search="onSearch" @clear="onSearchClear">
      <template #filters>
        <n-select
          v-model:value="roleFilter"
          :options="roleFilterOptions"
          placeholder="按角色筛选"
          clearable
          style="width: 160px"
        />
        <n-select
          v-model:value="statusFilter"
          :options="statusFilterOptions"
          placeholder="按状态筛选"
          clearable
          style="width: 130px"
        />
      </template>
      <template #actions>
        <n-space :size="8">
          <n-button @click="fetchUsers">查询</n-button>
          <PermissionButton :permission="PERM.USER_CREATE" @click="openImport">
            批量导入
          </PermissionButton>
        </n-space>
      </template>
    </SearchForm>

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
          <n-input
            v-model:value="formData.username"
            :placeholder="USERNAME_PLACEHOLDER"
            :maxlength="USERNAME_MAX_LEN"
          />
        </n-form-item>
        <n-form-item label="邮箱" path="email">
          <n-input v-model:value="formData.email" placeholder="请输入邮箱地址" :maxlength="255" />
        </n-form-item>
        <n-form-item v-if="!isEditing" label="密码" path="password">
          <n-input
            v-model:value="formData.password"
            type="password"
            :placeholder="PASSWORD_PLACEHOLDER"
            :maxlength="PASSWORD_MAX_LEN"
          />
        </n-form-item>
        <n-form-item label="角色" path="roles">
          <n-select
            v-model:value="formData.roles"
            multiple
            clearable
            :options="roleOptions"
            :disabled="!!rolesUnavailable"
            :placeholder="rolesUnavailable ? '角色列表不可用' : '请选择角色（可多选）'"
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

    <!-- 在线会话抽屉 -->
    <n-drawer
      v-model:show="showSessions"
      :width="560"
      :title="sessionsTitle"
      placement="right"
    >
      <n-spin :show="sessionsLoading">
        <n-alert v-if="sessions.length === 0 && !sessionsLoading" type="default" :show-icon="false">
          该用户当前没有在线会话。
        </n-alert>
        <n-list v-else hoverable>
          <n-list-item v-for="s in sessions" :key="s.jti">
            <n-thing>
              <template #header>
                {{ s.client_ip }}
                <n-tag v-if="s.is_current" size="small" type="success" :bordered="false">
                  当前会话
                </n-tag>
              </template>
              <template #description>
                登录于 {{ formatTime(s.login_at_ms) }} · 到期于 {{ formatTime(s.expires_at_ms) }}
              </template>
              <template #header-extra>
                <n-button
                  v-if="!s.is_current"
                  size="tiny"
                  type="error"
                  quaternary
                  :loading="revokingJti === s.jti"
                  @click="handleRevoke(s.jti)"
                >
                  吊销
                </n-button>
              </template>
            </n-thing>
          </n-list-item>
        </n-list>
      </n-spin>
    </n-drawer>

    <!-- CSV 批量导入 -->
    <n-modal
      v-model:show="showImport"
      title="从 CSV 批量导入用户"
      :mask-closable="false"
      preset="card"
      style="width: 640px"
    >
      <n-alert type="info" :show-icon="false" style="margin-bottom: 12px">
        表头必需列：<code>username,email,password,roles</code>；
        可选列：<code>display_name</code>。角色多个时用 <code>|</code> 分隔，
        例如 <code>user|admin</code>。
      </n-alert>
      <n-input
        v-model:value="importCsv"
        type="textarea"
        :rows="8"
        placeholder="username,email,password,roles&#10;alice,alice@example.com,Pw123456!,user"
      />
      <div style="margin-top: 12px">
        <n-checkbox v-model:checked="importDryRun">试运行（只校验不落库）</n-checkbox>
      </div>

      <!-- 逐行失败原因。**必须展示**：200 只代表请求成功，不代表每一行都建成了 -->
      <n-alert
        v-if="importResult"
        :type="importResult.failed > 0 ? 'warning' : 'success'"
        style="margin-top: 12px"
      >
        共 {{ importResult.total }} 行：成功 {{ importResult.created }}，失败
        {{ importResult.failed }}{{ importResult.dry_run ? '（试运行，未落库）' : '' }}
      </n-alert>
      <n-table v-if="importResult && importResult.failures.length > 0" size="small" style="margin-top: 8px">
        <thead>
          <tr><th>行号</th><th>用户名</th><th>原因</th></tr>
        </thead>
        <tbody>
          <tr v-for="f in importResult.failures" :key="f.line">
            <td>{{ f.line }}</td>
            <td>{{ f.username || '—' }}</td>
            <td>{{ f.reason }}</td>
          </tr>
        </tbody>
      </n-table>

      <template #footer>
        <n-space justify="end">
          <n-button @click="showImport = false">关闭</n-button>
          <n-button type="primary" :loading="importing" :disabled="!importCsv.trim()" @click="runImport">
            开始导入
          </n-button>
        </n-space>
      </template>
    </n-modal>
  </div>
</template>

<script setup lang="ts">
import { computed, ref, h, onMounted, watch } from 'vue'
import { NImage, NTag, NSwitch } from 'naive-ui'
import { AddOutline as AddIcon } from '@vicons/ionicons5'
import type { DataTableColumn, FormInst, FormRules, SelectOption } from 'naive-ui'
import type { ImportUsersResult, UserInfo, UserSession } from '@/api/types/response'
import { displayLabel } from '@/api/types/response'
import { userApi } from '@/api/user'
import { showConfirm, showSuccess, showWarning } from '@/utils/message'
import BaseTable from '@/components/common/BaseTable.vue'
import SearchForm from '@/components/common/SearchForm.vue'
import PermissionButton from '@/components/common/PermissionButton.vue'
import { PERM } from '@/constants/permission'
import { ADMIN_ROLE_NAME } from '@/constants/builtin'
import { roleApi } from '@/api/role'
import type { RoleListItem, RoleSelectOption } from '@/utils/role'
import {
  buildRoleSelectOptions,
  currentRoleNames,
  pickDefaultRoles,
} from '@/utils/role'
import {
  emailRules,
  PASSWORD_PLACEHOLDER,
  passwordPolicyRules,
  USERNAME_MAX_LEN,
  USERNAME_PLACEHOLDER,
  usernameRules,
} from '@/utils/accountRules'
import { PASSWORD_MAX_LEN } from '@/utils/password'

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
/** 搜索关键字：同时匹配用户名与邮箱 */
const keyword = ref('')
/**
 * 角色筛选（v0.20.0）
 *
 * 与 `is_active` 分开成两个独立条件，而不是塞进一个"高级筛选"弹窗：
 * 管理员找人的真实问题是"那个角色的禁用账号还有谁"，
 * 两个下拉并排才让这个问题能被一眼打完。
 */
const roleFilter = ref<string | null>(null)
/**
 * 激活状态筛选
 *
 * 存的是**字符串**而不是 boolean：naive-ui 的 `SelectOption.value` 类型是
 * `string | number`，传 boolean 只能靠类型断言绕过，而那样筛选下拉
 * 的选中态在某些版本下会显示不出来。
 * 真正发给后端时再转成 boolean（见 `fetchUsers`）。
 */
const statusFilter = ref<string | null>(null)
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
    // 必须取全量：下拉框只显示第一页的话，用户看不到自己实际持有的角色
    availableRoles.value = await roleApi.listAll()
  } catch {
    rolesUnavailable.value = '无法读取角色列表，请检查是否具备角色查看权限'
  }
}

interface UserForm {
  username: string
  email: string
  password: string
  /** 权威字段：多角色。v0.6.0 之前是单数 `role`，见后端 UserManageRequest */
  roles: string[]
  is_active: boolean
}

const formData = ref<UserForm>({
  username: '',
  email: '',
  password: '',
  roles: [],
  is_active: true,
})

// 用户名/邮箱/口令三组规则来自 `@/utils/accountRules`，与注册页、改密页同源。
//
// v0.18.0 之前这里是自己写的：`password: [{ min: 6 }]` 且**完全没有用户名字符集
// 规则**。管理员在对话框里输入 `user@name` 能过前端校验，保存后才收到
// "用户名只能包含字母、数字、下划线和连字符"。
const formRules: FormRules = {
  username: usernameRules,
  email: emailRules,
  password: passwordPolicyRules,
  // 多选：naive-ui 的 `required` 对数组不生效，必须配 `type:'array'` + `min`
  roles: [
    { required: true, type: 'array', min: 1, message: '请至少选择一个角色' },
  ],
}

// ── 筛选选项 ────────────────────────────────────────────────────

const statusFilterOptions: SelectOption[] = [
  { label: '仅启用', value: 'true' },
  { label: '仅禁用', value: 'false' },
]

/**
 * 角色筛选下拉的选项
 *
 * 与建号对话框共用同一份"按需加载"结果，但**不阻塞列表加载**：
 * 只浏览列表的账号可能没有 `system:role:list`，那时下拉为空而不是报错。
 */
const roleFilterOptions = computed<SelectOption[]>(() =>
  availableRoles.value.map((r) => ({ label: r.name, value: r.name })),
)

// ── 表格列 ──────────────────────────────────────────────────────

/**
 * 头像展示 URL
 *
 * `avatar_url` 是站内相对路径，直接当 `src` 会打到前端 dev server 上，
 * 于是头像永远裂开——这是"后端存对了但界面全错"的典型形态。
 */
function avatarUrlOf(user: UserInfo): string {
  const url = user.avatar_url
  if (!url) return ''
  return `${import.meta.env.VITE_API_BASE_URL || '/api'}${url}`
}

const columns: DataTableColumn[] = [
  {
    title: '用户',
    key: 'username',
    width: 200,
    render(row: Record<string, unknown>) {
      const r = row as unknown as UserInfo
      return h('div', { style: 'display:flex;align-items:center;gap:8px' }, [
        h(NImage, {
          src: avatarUrlOf(r),
          alt: displayLabel(r),
          width: 28,
          height: 28,
          objectFit: 'cover',
          // 没头像时不要放一个碎图占位：一个 28px 的灰色方块比不显示更像故障
          fallbackSrc: '',
        }),
        h('div', { style: 'display:flex;flex-direction:column;line-height:1.3' }, [
          h('span', null, displayLabel(r)),
          // 展示名与用户名不同才显示后者，否则同一串字出现两遍
          r.display_name && r.display_name !== r.username
            ? h('span', { style: 'font-size:12px;color:#999' }, r.username)
            : null,
        ]),
      ])
    },
  },
  { title: '邮箱', key: 'email', width: 200 },
  {
    title: '角色',
    key: 'roles',
    width: 160,
    render(row: Record<string, unknown>) {
      const r = row as unknown as UserInfo
      // 显示**全部**真实角色：r.role 是两值枚举，自定义角色会被塌缩成 "user"，
      // 且 v0.6.0 之前只渲染 roles[0]，多角色用户看起来就是单角色——
      // 管理员根本看不出这个人其实还持有别的权限
      const names = currentRoleNames(r)
      return h(
        'div',
        { style: 'display:flex;gap:4px;flex-wrap:wrap' },
        names.map((name) =>
          h(
            NTag,
            {
              type: name === ADMIN_ROLE_NAME ? 'warning' : 'info',
              size: 'small',
            },
            () => name,
          ),
        ),
      )
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
      return h('div', { style: 'display:flex;gap:8px;flex-wrap:wrap' }, [
        h(PermissionButton, { permission: PERM.USER_UPDATE, size: 'small', onClick: () => openEdit(r) }, () => '编辑'),
        h(PermissionButton, { permission: PERM.USER_UNLOCK, size: 'small', onClick: () => handleUnlock(r) }, () => '解锁'),
        h(PermissionButton, { permission: PERM.SESSION_MANAGE, size: 'small', onClick: () => openSessions(r) }, () => '会话'),
        h(PermissionButton, { permission: PERM.USER_DELETE, size: 'small', type: 'error', onClick: () => handleDelete(r.id) }, () => '删除'),
      ])
    },
  },
]

// ── 数据加载 ────────────────────────────────────────────────────

async function fetchUsers() {
  loading.value = true
  try {
    const res = await userApi.list({
      page: page.value,
      page_size: pageSize.value,
      keyword: keyword.value || undefined,
      // 后端 DTO 开了 deny_unknown_fields：undefined 会让 axios 省略该项，
      // 而传 null 会真的发出 "is_active=null"，被 400 拒掉
      role: roleFilter.value || undefined,
      is_active: statusFilter.value === null ? undefined : statusFilter.value === 'true',
    })
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

function onSearch(value: string) {
  keyword.value = value
  page.value = 1
  fetchUsers()
}

function onSearchClear() {
  keyword.value = ''
  page.value = 1
  fetchUsers()
}

/**
 * 筛选变化后回到第 1 页
 *
 * 停在第 5 页再改筛选，那个页码在新结果集上往往已越界，
 * 于是用户看到的是"筛选一改就空了"，而不是"筛选生效了"。
 */
watch([roleFilter, statusFilter], () => {
  page.value = 1
  fetchUsers()
})

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
    roles: pickDefaultRoles(availableRoles.value),
    is_active: true,
  }
  showModal.value = true
}

async function openEdit(user: UserInfo) {
  await ensureRolesLoaded()
  isEditing.value = true
  editingId.value = user.id
  // 必须取**全部**真实角色，不能用 user.role：后者是非 admin 一律塌缩成 "user"
  // 的展示枚举；也不能只取 roles[0]——那会把其余角色静默删掉
  const current = currentRoleNames(user)
  // 带上全部当前角色：任一可能不在列表里（历史遗留的非归一化角色名）
  roleOptions.value = buildRoleSelectOptions(availableRoles.value, current)
  formData.value = {
    username: user.username,
    email: user.email,
    password: '',
    roles: current,
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
        roles: formData.value.roles,
        is_active: formData.value.is_active,
      })
      showSuccess('用户更新成功')
    } else {
      await userApi.create({
        username: formData.value.username,
        email: formData.value.email,
        password: formData.value.password || undefined,
        roles: formData.value.roles,
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

// ── 解锁（v0.20.0）──────────────────────────────────────────────

/**
 * 解锁账号
 *
 * **必须先确认**：解锁等于认定"之前那些失败登录是用户本人而不是爆破"，
 * 后端会清掉该账号的登录失败计数——若那是正在进行的爆破，
 * 这一下就送给攻击者一份新额度。
 */
async function handleUnlock(user: UserInfo) {
  const confirmed = await showConfirm({
    content: `确定解锁「${displayLabel(user)}」？这会清除其登录失败计数，请先确认不是口令爆破。`,
  })
  if (!confirmed) return

  try {
    const res = (await userApi.unlock(user.id)) as unknown as {
      cleared_failures: number
      scopes_cleared: number
    }
    // 说清清了什么：回一句"解锁成功"时，管理员分不清"确实锁过"
    // 与"这个账号本来就能登录"，也就无法判断解锁是否真的起了作用
    showSuccess(
      res.cleared_failures > 0
        ? `已解锁，清除 ${res.cleared_failures} 次失败计数`
        : '该账号当前没有被锁定，无需解锁',
    )
  } catch {
    // handled
  }
}

// ── 在线会话（v0.20.0）──────────────────────────────────────────

const showSessions = ref(false)
const sessionsLoading = ref(false)
const sessions = ref<UserSession[]>([])
const sessionsUser = ref<UserInfo | null>(null)
const revokingJti = ref('')

const sessionsTitle = computed(
  () => `在线会话 — ${sessionsUser.value ? displayLabel(sessionsUser.value) : ''}`,
)

function formatTime(ms: number): string {
  const d = new Date(ms)
  return Number.isNaN(d.getTime()) ? '—' : d.toLocaleString()
}

async function openSessions(user: UserInfo) {
  sessionsUser.value = user
  sessions.value = []
  showSessions.value = true
  sessionsLoading.value = true
  try {
    const res = (await userApi.sessions(user.id)) as unknown as UserSession[]
    sessions.value = res
  } catch {
    // handled
  } finally {
    sessionsLoading.value = false
  }
}

/**
 * 吊销单个会话
 *
 * **不允许吊销"当前会话"**：管理员多半正是在自己的浏览器里点的，
 * 吊销掉会把自己踢出系统，而这不是他这次点击想要的结果。
 */
async function handleRevoke(jti: string) {
  if (!sessionsUser.value) return
  const confirmed = await showConfirm({ content: '确定吊销该会话？该设备需重新登录。' })
  if (!confirmed) return

  revokingJti.value = jti
  try {
    await userApi.revokeSession(sessionsUser.value.id, jti)
    showSuccess('会话已吊销')
    sessions.value = sessions.value.filter((s) => s.jti !== jti)
  } catch {
    // handled
  } finally {
    revokingJti.value = ''
  }
}

// ── CSV 批量导入（v0.20.0）──────────────────────────────────────

const showImport = ref(false)
const importing = ref(false)
const importCsv = ref('')
/** 默认试运行：导入的真实风险是"建了一百个号发现全部要返工" */
const importDryRun = ref(true)
const importResult = ref<ImportUsersResult | null>(null)

function openImport() {
  importCsv.value = ''
  importResult.value = null
  importDryRun.value = true
  showImport.value = true
}

async function runImport() {
  if (importing.value) return
  importing.value = true
  importResult.value = null
  try {
    const res = (await userApi.importUsers(importCsv.value, importDryRun.value)) as unknown as ImportUsersResult
    importResult.value = res
    if (res.failed === 0) {
      showSuccess(res.dry_run ? `试运行通过，${res.created} 行均可导入` : `成功导入 ${res.created} 个用户`)
      if (!res.dry_run) fetchUsers()
    } else {
      // 部分失败必须用 warning 而不是 error：接口本身是成功的
      showWarning(`${res.failed} 行未通过，其余 ${res.created} 行${res.dry_run ? '可导入' : '已导入'}`)
    }
  } catch {
    // handled
  } finally {
    importing.value = false
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
