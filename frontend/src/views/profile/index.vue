<template>
  <div class="page-container">
    <n-page-header
      title="个人中心"
      subtitle="维护本人资料与登录密码"
    />

    <!-- 受限令牌：把"为什么只能待在这一页"说清楚，而不是让用户自己撞 403 -->
    <n-alert
      v-if="userStore.mustChangePassword"
      type="warning"
      title="需要先修改初始密码"
      class="force-alert"
    >
      该密码由管理员设置或重置。在完成修改前，除改密与登出外的功能都不可用。
    </n-alert>

    <n-grid :cols="2" :x-gap="16" :y-gap="16" responsive="screen" item-responsive>
      <n-grid-item span="2 m:1">
        <n-card title="账号信息" size="small">
          <n-descriptions :column="1" label-placement="left" bordered size="small">
            <n-descriptions-item label="用户名">
              {{ userStore.userInfo?.username || '—' }}
            </n-descriptions-item>
            <n-descriptions-item label="展示名">
              {{ userStore.userInfo?.display_name || '未设置' }}
              <div class="hint">
                列表与页面上优先显示这一项；留空则回退为用户名。
              </div>
            </n-descriptions-item>
            <n-descriptions-item label="头像">
              <n-avatar
                v-if="avatarSrc"
                :src="avatarSrc"
                :alt="userStore.userInfo?.display_name || userStore.userInfo?.username"
                round
                size="small"
                style="margin-right: 8px"
              />
              <span v-else class="hint">未设置</span>
              <BaseUpload
                mode="button"
                list-type="text"
                accept="image/png,image/jpeg,image/webp,image/gif"
                :max-size="avatarMaxSize"
                :action="uploadAction"
                @success="onAvatarUploaded"
                @error="onAvatarFailed"
              />
            </n-descriptions-item>
            <n-descriptions-item label="邮箱">
              {{ userStore.userInfo?.email || '—' }}
            </n-descriptions-item>
            <n-descriptions-item label="角色">
              <n-space :size="4">
                <n-tag v-for="r in roles" :key="r" size="small" :bordered="false">
                  {{ r }}
                </n-tag>
                <span v-if="roles.length === 0">—</span>
              </n-space>
            </n-descriptions-item>
            <n-descriptions-item label="状态">
              <n-tag
                size="small"
                :bordered="false"
                :type="userStore.userInfo?.is_active ? 'success' : 'error'"
              >
                {{ userStore.userInfo?.is_active ? '正常' : '已停用' }}
              </n-tag>
            </n-descriptions-item>
            <n-descriptions-item label="注册时间">
              {{ createdAt }}
            </n-descriptions-item>
          </n-descriptions>
        </n-card>
      </n-grid-item>

      <n-grid-item span="2 m:1">
        <n-card title="展示名" size="small">
          <n-form
            ref="profileFormRef"
            :model="profileForm"
            :rules="profileRules"
            label-placement="left"
            label-width="80px"
          >
            <n-form-item label="展示名" path="displayName">
              <n-input
                v-model:value="profileForm.displayName"
                placeholder="例如：张三"
                :maxlength="DISPLAY_NAME_MAX_LEN"
                clearable
              />
            </n-form-item>
            <n-space align="center">
              <n-button
                type="primary"
                :loading="profileSaving"
                :disabled="!profileDirty"
                @click="saveProfile"
              >
                保存
              </n-button>
              <n-button v-if="profileDirty" quaternary @click="resetProfileForm">
                放弃修改
              </n-button>
            </n-space>
          </n-form>
          <div class="hint" style="margin-top: 8px">
            留空即清空。展示名只影响显示，登录仍用用户名或邮箱。
          </div>
        </n-card>
      </n-grid-item>

      <n-grid-item span="2 m:1">
        <n-card title="修改密码" size="small">
          <n-form ref="formRef" :model="form" :rules="rules" @submit.prevent="submit">
            <n-form-item label="当前密码" path="oldPassword">
              <n-input
                v-model:value="form.oldPassword"
                type="password"
                show-password-on="click"
                placeholder="请输入当前登录密码"
                @keyup.enter="submit"
              />
            </n-form-item>
            <n-form-item label="新密码" path="newPassword">
              <n-input
                v-model:value="form.newPassword"
                type="password"
                show-password-on="click"
                :placeholder="passwordPlaceholderText"
                :maxlength="passwordMaxLen"
                @keyup.enter="submit"
              />
            </n-form-item>
            <n-form-item label="确认新密码" path="confirmPassword">
              <n-input
                v-model:value="form.confirmPassword"
                type="password"
                show-password-on="click"
                placeholder="再次输入新密码"
                @keyup.enter="submit"
              />
            </n-form-item>
            <n-button
              type="primary"
              attr-type="submit"
              :loading="submitting"
              :disabled="!canSubmit"
            >
              确认修改
            </n-button>
          </n-form>
        </n-card>
      </n-grid-item>

      <n-grid-item span="2 m:1">
        <n-card title="登录会话" size="small">
          <template #header-extra>
            <n-space :size="8">
              <n-button
                quaternary
                size="small"
                :loading="sessionsLoading"
                @click="loadSessions"
              >
                <template #icon>
                  <n-icon><RefreshOutline /></n-icon>
                </template>
                刷新
              </n-button>
              <n-popconfirm @positive-click="revokeOthers">
                <template #trigger>
                  <n-button quaternary size="small" type="warning">
                    <template #icon>
                      <n-icon><LogOutOutline /></n-icon>
                    </template>
                    下线其他所有设备
                  </n-button>
                </template>
                将吊销除本设备外的全部会话，且**本设备不受影响**。确认继续？
              </n-popconfirm>
            </n-space>
          </template>

          <n-spin :show="sessionsLoading">
            <n-list v-if="sessions.length > 0" bordered>
              <n-list-item v-for="s in sessions" :key="s.jti">
                <n-space align="center" :size="12" style="width: 100%">
                  <n-tag
                    v-if="s.is_current"
                    size="small"
                    type="success"
                    :bordered="false"
                  >
                    本设备
                  </n-tag>
                  <n-tag v-else size="small" :bordered="false">其他设备</n-tag>

                  <n-space vertical :size="0" style="flex: 1">
                    <span class="session-ip">{{ s.client_ip }}</span>
                    <span class="hint">
                      登录于 {{ formatMs(s.login_at_ms) }} ·
                      到期 {{ formatMs(s.expires_at_ms) }}
                    </span>
                  </n-space>

                  <n-button
                    quaternary
                    size="small"
                    type="error"
                    :loading="revokingJti === s.jti"
                    @click="revokeOne(s)"
                  >
                    下线
                  </n-button>
                </n-space>
              </n-list-item>
            </n-list>
            <n-empty v-else description="当前没有在线会话" size="small" />
          </n-spin>

          <div class="hint" style="margin-top: 8px">
            发现不认识的设备？先「下线」它，再改密码。改密会吊销**全部**会话，
            包括本设备；「下线其他所有设备」则只踢掉别的设备。
          </div>
        </n-card>
      </n-grid-item>

      <!--
        两步验证（v0.25.0）

        放在登录会话之后而不是账号信息里：它是"给自己加一道锁"，
        和会话管理同属账号安全，混进基本资料里容易被当成一个字段忽略。
      -->
      <n-grid-item span="2 m:1">
        <TwoFactorCard />
      </n-grid-item>
    </n-grid>
  </div>
</template>

<script setup lang="ts">
/**
 * 个人中心
 *
 * 提供自助改密。此前用户只能等管理员重置才知道该改密码。
 *
 * **改密成功后会退出登录**：后端吊销了该用户的全部会话（含本端），
 * 继续留在页面上只会在下一次请求时收到 401。前端主动登出并给出提示，
 * 好过让用户莫名其妙地被踢回登录页。
 *
 * 口令规则与后端 `utils::validation::validate_password` 一致。
 * 这里重复实现一遍是为了即时反馈，**后端仍是唯一裁决方**——
 * 前端校验只是体验，不是安全边界。
 */
import { computed, onMounted, reactive, ref } from 'vue'
import type { FormInst, FormRules, UploadFileInfo } from 'naive-ui'
import { LogOutOutline, RefreshOutline } from '@vicons/ionicons5'
import { useRouter } from 'vue-router'
import { useUserStore } from '@/stores/user'
import { authApi } from '@/api/auth'
import { showError, showSuccess, showWarning } from '@/utils/message'
import { passwordIssues } from '@/utils/password'
import { passwordMaxLength, passwordPlaceholder } from '@/utils/accountRules'
import { useSettingStore } from '@/stores/setting'
import BaseUpload from '@/components/common/BaseUpload.vue'
import TwoFactorCard from '@/components/security/TwoFactorCard.vue'
import { resolveAvatarUrl } from '@/utils/avatar'
import type { UserInfo, UserSession } from '@/api/types/response'

const router = useRouter()
const userStore = useUserStore()

const roles = computed(() => userStore.userInfo?.roles ?? [])

// ── 头像 ────────────────────────────────────────────────────────

/**
 * 上传端点的完整 URL
 *
 * BaseUpload 走的是 naive-ui 的原生 XHR，**不经过 axios 实例**，
 * 所以它拿不到 `http.defaults.baseURL`，必须自己拼全路径。
 */
const uploadAction = `${import.meta.env.VITE_API_BASE_URL || '/api'}/auth/profile/avatar`

/** 与后端 `UploadConfig::max_file_size` 默认值一致（2MB） */
const avatarMaxSize = 2 * 1024 * 1024

/**
 * 头像展示 URL
 *
 * 拼接规则（站内相对路径要补 API 前缀）收在 `@/utils/avatar` 里，
 * 顶栏头像与这里走同一个函数——两处各写一份时，改了一处就会留下
 * "个人中心能看到头像、顶栏还是首字母"的分裂。
 */
const avatarSrc = computed(() => resolveAvatarUrl(userStore.userInfo?.avatar_url))

/**
 * naive-ui 的上传组件直接把响应体塞进 `file.response`，
 * 且**不走 axios 响应拦截器**，所以 `{code, message, data}` 信封要自己拆。
 */
function onAvatarUploaded(file: UploadFileInfo) {
  // naive-ui 的 UploadFileInfo 类型里没有 `response`，但原生 XHR 完成后确实会挂上
  const body = (file as unknown as { response?: unknown }).response as
    | { code?: number; message?: string; data?: { url?: string } }
    | undefined
  if (body?.code !== 200 || !body?.data?.url) {
    showError(body?.message || '头像上传失败')
    return
  }
  // 重新拉一次而不是本地乐观更新：服务端可能归一过值，
  // 界面上显示一个库里没有的名字比晚 200ms 更糟
  userStore
    .fetchUserInfo()
    .then(() => showSuccess('头像已更新'))
    .catch(() => {
      // 错误提示已由 store 内部处理
    })
}

function onAvatarFailed(file: UploadFileInfo) {
  const body = (file as unknown as { response?: unknown }).response as
    | { message?: string }
    | undefined
  showError(body?.message || '头像上传失败')
}

// ── 登录会话（v0.23.0）─────────────────────────────────────────

const sessions = ref<UserSession[]>([])
const sessionsLoading = ref(false)
const revokingJti = ref('')

/**
 * 把 Unix 毫秒格式化成可读时间
 *
 * 解析失败时回退成原值而不是 `Invalid Date`：
 * 后端给的是数字，一旦格式变化，显示 `Invalid Date` 比显示原始数字更糟。
 */
function formatMs(ms: number) {
  const d = new Date(ms)
  return Number.isNaN(d.getTime()) ? String(ms) : d.toLocaleString()
}

async function loadSessions() {
  if (sessionsLoading.value) return
  sessionsLoading.value = true
  try {
    sessions.value = (await authApi.mySessions()) as unknown as UserSession[]
  } catch {
    // 错误提示已由响应拦截器统一弹出
  } finally {
    sessionsLoading.value = false
  }
}

/**
 * 下线单个会话
 *
 * **允许下线当前会话**（管理端刻意禁止）：用户是主动点"下线这台设备"，
 * 紧接着的 401 正是他想要的结果。但下线本设备后页面会在下一次请求时
 * 被踢回登录页，所以先给一句明确提示。
 */
async function revokeOne(s: UserSession) {
  if (revokingJti.value) return
  revokingJti.value = s.jti
  try {
    await authApi.revokeMySession(s.jti)
    if (s.is_current) {
      // 本设备被自己下线：后端已把令牌拉黑，这里主动登出，
      // 好过让用户在下一次请求时莫名其妙被踢回登录页
      showWarning('本设备已下线，即将返回登录页')
      await new Promise((r) => setTimeout(r, 800))
      // **不能调 `userStore.logout()`**：令牌已被自己拉黑，
      // 再发一次登出请求必然 401——一次注定失败的往返，
      // 外加控制台里一条 "Failed to load resource"，把真实错误淹掉。
      // 与改密成功后的处理同一套理由。
      userStore.clearLocalSession()
      router.push('/login')
      return
    }
    showSuccess('该设备已下线')
    await loadSessions()
  } catch {
    // 错误提示已由响应拦截器统一弹出
  } finally {
    revokingJti.value = ''
  }
}

/**
 * 下线除本设备外的全部会话
 *
 * "账号可能被盗用"时最该有的一台开关：改密会吊销全部会话（含本端），
 * 而"只踢掉其他设备、让我继续用"在语义上更准确——正在用的这台就是可信的证据。
 */
async function revokeOthers() {
  try {
    const result = (await authApi.revokeMyOtherSessions()) as unknown as {
      revoked_count: number
    }
    showSuccess(
      result.revoked_count > 0
        ? `已下线其他 ${result.revoked_count} 台设备`
        : '没有其他在线设备',
    )
    await loadSessions()
  } catch {
    // 错误提示已由响应拦截器统一弹出
  }
}

// ── 展示名 ──────────────────────────────────────────────────────

const DISPLAY_NAME_MAX_LEN = 50

const profileFormRef = ref<FormInst | null>(null)
const profileSaving = ref(false)
const profileForm = reactive({ displayName: '' })

/** 原值：用于判定"有没有改动"，避免每次都发一个空请求 */
const savedDisplayName = ref('')

const profileRules: FormRules = {
  displayName: [
    {
      trigger: ['input', 'blur'],
      validator: (_rule, value: string) =>
        (value ?? '').length <= DISPLAY_NAME_MAX_LEN
          ? true
          : new Error(`展示名不能超过 ${DISPLAY_NAME_MAX_LEN} 个字符`),
    },
  ],
}

const profileDirty = computed(() => profileForm.displayName !== savedDisplayName.value)

function resetProfileForm() {
  profileForm.displayName = savedDisplayName.value
}

async function saveProfile() {
  if (profileSaving.value) return
  try {
    await profileFormRef.value?.validate()
  } catch {
    return
  }
  profileSaving.value = true
  try {
    const info = (await authApi.updateProfile({
      // 空串而非 null：后端反序列化器把两者都归一成"清空"，
      // 传 null 时 JSON 里保留 null 字段，语义同样正确，
      // 用空串是为了让"用户清掉了"这件事在日志里也读得出来
      display_name: profileForm.displayName.trim(),
    })) as unknown as UserInfo
    userStore.applyUserInfo(info)
    savedDisplayName.value = info.display_name ?? ''
    profileForm.displayName = savedDisplayName.value
    showSuccess(profileForm.displayName ? '展示名已更新' : '展示名已清空')
  } catch {
    // 错误提示已由响应拦截器统一弹出
  } finally {
    profileSaving.value = false
  }
}

onMounted(() => {
  savedDisplayName.value = userStore.userInfo?.display_name ?? ''
  profileForm.displayName = savedDisplayName.value
  void settingStore.loadPasswordPolicy()
  void loadSessions()
})

const createdAt = computed(() => {
  const raw = userStore.userInfo?.created_at
  if (!raw) return '—'
  const d = new Date(raw)
  return Number.isNaN(d.getTime()) ? raw : d.toLocaleString()
})

const formRef = ref<FormInst | null>(null)
const submitting = ref(false)
const form = reactive({ oldPassword: '', newPassword: '', confirmPassword: '' })

// 新口令规则来自服务端（`GET /api/settings/password-policy`）。
// **「当前密码」不套策略**：它不是新设的口令，后端登录校验验的是
// Argon2 哈希、压根不看明文复杂度，给它挂策略会在策略被抬高后
// 把存量弱口令用户锁在改密页外——而他们连旧密码都提交不了。
const settingStore = useSettingStore()
const passwordPlaceholderText = computed(() => passwordPlaceholder(settingStore.passwordPolicy))
const passwordMaxLen = computed(() => passwordMaxLength(settingStore.passwordPolicy))

const rules = computed<FormRules>(() => ({
  oldPassword: [{ required: true, message: '请输入当前密码', trigger: ['input', 'blur'] }],
  newPassword: [
    { required: true, message: '请输入新密码', trigger: ['input', 'blur'] },
    {
      trigger: ['input', 'blur'],
      validator: (_rule, value: string) => {
        const problems = passwordIssues(value ?? '', settingStore.passwordPolicy)
        // naive-ui 的 validator 返回 Error 对象，不是字符串
        return problems.length === 0 ? true : new Error(problems.join('；'))
      },
    },
  ],
  confirmPassword: [
    { required: true, message: '请再次输入新密码', trigger: ['input', 'blur'] },
    {
      trigger: ['input', 'blur'],
      validator: (_rule, value: string) =>
        value === form.newPassword ? true : new Error('两次输入的新密码不一致'),
    },
  ],
}))

const canSubmit = computed(
  () =>
    !submitting.value &&
    form.oldPassword.length > 0 &&
    form.newPassword.length > 0 &&
    passwordIssues(form.newPassword, settingStore.passwordPolicy).length === 0 &&
    form.newPassword === form.confirmPassword,
)

async function submit() {
  if (submitting.value) return
  try {
    await formRef.value?.validate()
  } catch {
    return // 校验未通过，naive-ui 已就位提示
  }

  // 新旧相同没有意义：后端也会拒，这里先拦住以免一次无谓往返
  if (form.newPassword === form.oldPassword) {
    showWarning('新密码不能与当前密码相同')
    return
  }

  submitting.value = true
  try {
    await authApi.changePassword({
      old_password: form.oldPassword,
      new_password: form.newPassword,
    })
    showSuccess('密码修改成功，请重新登录')
    // **不能调 logout()**：改密成功时后端已吊销了该用户的全部会话，
    // 再发一次登出请求必然 401——一次注定失败的往返，
    // 外加控制台里一条 "Failed to load resource"，把真实错误淹掉
    userStore.clearLocalSession()
    await router.push('/login')
  } catch {
    // 错误提示已由响应拦截器统一弹出
  } finally {
    submitting.value = false
  }
}
</script>

<style scoped>
.page-container {
  padding: 16px;
}

.force-alert {
  margin-bottom: 16px;
}

.hint {
  font-size: 12px;
  color: var(--n-text-color-3, #999);
  margin-top: 4px;
}

.session-ip {
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  font-size: 13px;
}
</style>
