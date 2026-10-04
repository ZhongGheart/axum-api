<template>
  <AuthShell>
    <div class="login">
      <!-- 会话失效原因：被踢回登录页时说明为什么，否则用户面对的是一个空登录框 -->
      <n-alert
        v-if="sessionEndedMessage"
        type="warning"
        :show-icon="true"
        class="login-alert"
      >
        {{ sessionEndedMessage }}
      </n-alert>

      <header class="login-header">
        <h1 class="login-title">登录 Axum Admin</h1>
        <p class="login-subtitle">使用你的账号登录</p>
      </header>

      <!-- 登录表单 -->
      <n-form
        ref="formRef"
        :model="formData"
        :rules="formRules"
        label-placement="top"
        size="large"
        @submit.prevent="handleLogin"
      >
        <n-form-item label="用户名" path="username">
          <n-input
            v-model:value="formData.username"
            placeholder="请输入用户名或邮箱"
            :maxlength="IDENTIFIER_MAX_LEN"
            clearable
            :input-props="{ autocomplete: 'username' }"
          >
            <template #prefix>
              <n-icon><UserIcon /></n-icon>
            </template>
          </n-input>
        </n-form-item>

        <n-form-item label="密码" path="password">
          <n-input
            v-model:value="formData.password"
            type="password"
            show-password-on="click"
            placeholder="请输入密码"
            :maxlength="128"
            clearable
            :input-props="{ autocomplete: 'current-password' }"
          >
            <template #prefix>
              <n-icon><LockIcon /></n-icon>
            </template>
          </n-input>
        </n-form-item>

        <!-- 记住用户名 & 去注册 -->
        <div class="login-options">
          <n-checkbox v-model:checked="rememberMe">记住用户名</n-checkbox>
          <router-link to="/register" class="login-link">没有账号？立即注册</router-link>
        </div>

        <n-button
          type="primary"
          block
          size="large"
          attr-type="submit"
          :loading="submitting"
        >
          登录
        </n-button>
      </n-form>

      <p class="login-footnote">口令经 HTTPS 提交，服务端只存 Argon2 哈希</p>
    </div>
  </AuthShell>
</template>

<script setup lang="ts">
/**
 * 登录页面
 *
 * - 表单规则取自共享校验器 `utils/accountRules.ts`
 *   （这个字段收的是"用户名或邮箱"，所以刻意不套用户名字符集规则；
 *    口令只判空——后端登录验的是 Argon2 哈希，不套明文复杂度策略）
 * - 记住登录：只记忆用户名，口令不落盘（所以界面上的复选框写"记住用户名"）
 * - 口令经 HTTPS 明文提交，后端 Argon2 校验
 */
import { ref, computed, onMounted } from 'vue'
import { useRouter } from 'vue-router'
import { PersonOutline as UserIcon, LockClosedOutline as LockIcon } from '@vicons/ionicons5'
import type { FormInst, FormRules } from 'naive-ui'
import { useUserStore } from '@/stores/user'
import { showSuccess } from '@/utils/message'
import { getStorage, setStorage, removeStorage } from '@/utils/storage'
import { buildSessionEndedMessage, takeSessionEnded } from '@/utils/session'
import { IDENTIFIER_MAX_LEN, loginIdentifierRules, loginPasswordRules } from '@/utils/accountRules'
import AuthShell from '@/components/common/AuthShell.vue'

// ── 状态 ────────────────────────────────────────────────────────

const router = useRouter()
const userStore = useUserStore()
const formRef = ref<FormInst | null>(null)
const submitting = ref(false)

/**
 * 为什么会落在这个页面上
 *
 * 会话被吊销/过期时，响应拦截器会记下**后端给的原因**（`令牌已被注销` /
 * `登录状态已失效` / `令牌无效或已过期`），由这里取走展示。
 *
 * 用后端原话而不是前端另编的通用句：后端分得清是哪一种，
 * 前端编一句"会话已失效"等于把这份区别重新抹平。
 * 没有原因时（如主动点"退出登录"、直接访问 /login）就不显示——
 * 常驻提示会让人以为自己的会话出了什么问题。
 */
const sessionEndedReason = ref<string | null>(null)

/** 提示全文（拼接规则与单测见 utils/session.buildSessionEndedMessage） */
const sessionEndedMessage = computed(() => buildSessionEndedMessage(sessionEndedReason.value))

/** 记住用户名的存储键 */
const REMEMBER_KEY = 'remember_login'

interface RememberData {
  /** 仅记忆用户名；口令不落盘，避免本地明文泄露 */
  username: string
}

interface LoginForm {
  username: string
  password: string
}

const formData = ref<LoginForm>({
  username: '',
  password: '',
})

const rememberMe = ref(false)

// ── 表单校验规则 ────────────────────────────────────────────────
//
// v0.18.0 之前这里是两份手写规则，与后端都对不上：
//
// - `min: 6`：v0.10 时代口令策略的化石。后端登录验 Argon2 哈希，
//   压根不看明文复杂度，这条规则没有任何依据（方向上是宽松的，
//   所以一直没锁死谁，但它不是"对齐后端"，注释里却这么写）
// - `max: 50`：**真缺陷**。这个字段收的是"用户名或邮箱"，而后端
//   `find_by_username_or_email` 是一条裸查询，`users.email` 列宽 255。
//   实测 73 字符的邮箱注册与登录都成功，但前端输入框只给 50 个字符——
//   持有长邮箱的合法用户在自己的登录页上敲不进自己的邮箱。
//   另：字符集规则也**不能**加，含 `@` 的邮箱会被自己人拦下。

const formRules: FormRules = {
  username: loginIdentifierRules,
  password: loginPasswordRules,
}

// ── 记住用户名 ──────────────────────────────────────────────────

function loadRemembered(): void {
  const saved = getStorage<RememberData>(REMEMBER_KEY)
  if (saved) {
    formData.value.username = saved.username
    rememberMe.value = true
  }
}

function saveRemembered(): void {
  if (rememberMe.value) {
    setStorage(REMEMBER_KEY, { username: formData.value.username })
  } else {
    removeStorage(REMEMBER_KEY)
  }
}

// ── 登录提交 ────────────────────────────────────────────────────

async function handleLogin(): Promise<void> {
  if (submitting.value) return
  submitting.value = true

  try {
    // 表单校验
    try {
      await formRef.value?.validate()
    } catch {
      submitting.value = false
      return
    }

    // 保存记住密码状态
    saveRemembered()

    // 调用 user store 登录（内部做 SHA-256 哈希）
    const result = await userStore.login({
      username: formData.value.username,
      password: formData.value.password,
    })

    if (result) {
      showSuccess('登录成功')
      router.push('/')
    }
  } catch (e) {
    console.error('登录失败:', e)
  } finally {
    submitting.value = false
  }
}

// ── 初始化 ──────────────────────────────────────────────────────

onMounted(() => {
  loadRemembered()
  // 取走即清除：留在 sessionStorage 里会让**下一次**正常登录也显示它
  sessionEndedReason.value = takeSessionEnded()
})
</script>

<style scoped>
.login {
  display: flex;
  flex-direction: column;
}

.login-alert {
  margin-bottom: 16px;
}

.login-header {
  margin-bottom: 24px;
}

/*
 * 标签从"左侧固定宽度"改为置于输入框上方：
 * 左边距着一列固定宽度的文字，窄屏下会把输入框挤窄，
 * 竖排读起来也更像登录表单而不是设置页。
 *
 * 按钮文案也从"登 录"（加空格凑字距）改回"登录"——
 * 靠字符间空格撑开字距是排版手法，复制粘贴时会带上多余的空白。
 */
.login-title {
  font-size: 20px;
  font-weight: 600;
  color: var(--text-primary);
}

.login-subtitle {
  margin-top: 4px;
  font-size: 13px;
  color: var(--text-secondary);
}

.login-options {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: 20px;
}

.login-link {
  font-size: 13px;
  color: var(--primary-color);
}

.login-link:hover {
  text-decoration: underline;
}

.login-footnote {
  margin-top: 20px;
  font-size: 12px;
  color: var(--text-tertiary);
  text-align: center;
}
</style>