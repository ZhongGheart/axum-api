<template>
  <div class="login-page">
    <div class="login-card">
      <!-- Logo & 标题 -->
      <div class="login-header">
        <div class="logo">
          <span class="logo-icon">⚡</span>
          <span class="logo-text">Axum Admin</span>
        </div>
        <p class="login-subtitle">企业级后台管理系统</p>
      </div>

      <!-- 登录表单 -->
      <n-form
        ref="formRef"
        :model="formData"
        :rules="formRules"
        label-placement="left"
        label-width="auto"
        size="large"
      >
        <n-form-item label="用户名" path="username">
          <n-input
            v-model:value="formData.username"
            placeholder="请输入用户名或邮箱"
            :maxlength="50"
            clearable
            @keyup.enter="handleLogin"
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
            @keyup.enter="handleLogin"
          >
            <template #prefix>
              <n-icon><LockIcon /></n-icon>
            </template>
          </n-input>
        </n-form-item>

        <!-- 记住密码 & 去注册 -->
        <div class="login-options">
          <n-checkbox v-model:checked="rememberMe">记住密码</n-checkbox>
          <router-link to="/register" class="register-link">还没有账号？去注册</router-link>
        </div>

        <n-button
          type="primary"
          block
          size="large"
          :loading="submitting"
          class="login-btn"
          @click="handleLogin"
        >
          登 录
        </n-button>
      </n-form>
    </div>
  </div>
</template>

<script setup lang="ts">
/**
 * 登录页面
 *
 * - 表单校验规则对齐后端（用户名 3-50 字符，密码至少 6 位）
 * - 记住登录：只记忆用户名，口令不落盘
 * - 口令经 HTTPS 明文提交，后端 Argon2 校验
 */
import { ref, onMounted } from 'vue'
import { useRouter } from 'vue-router'
import { PersonOutline as UserIcon, LockClosedOutline as LockIcon } from '@vicons/ionicons5'
import type { FormInst, FormRules } from 'naive-ui'
import { useUserStore } from '@/stores/user'
import { showSuccess } from '@/utils/message'
import { getStorage, setStorage, removeStorage } from '@/utils/storage'

// ── 状态 ────────────────────────────────────────────────────────

const router = useRouter()
const userStore = useUserStore()
const formRef = ref<FormInst | null>(null)
const submitting = ref(false)

/** 记住密码标识 */
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

// ── 表单校验规则（对齐后端 service/auth.rs） ──────────────────────

const formRules: FormRules = {
  username: [
    { required: true, message: '请输入用户名或邮箱', trigger: 'blur' },
    { min: 3, message: '用户名至少 3 个字符', trigger: 'blur' },
    { max: 50, message: '用户名不能超过 50 个字符', trigger: 'blur' },
  ],
  password: [
    { required: true, message: '请输入密码', trigger: 'blur' },
    { min: 6, message: '密码至少 6 个字符', trigger: 'blur' },
  ],
}

// ── 记住密码 ────────────────────────────────────────────────────

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
})
</script>

<style scoped>
.login-page {
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: 100vh;
  background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
}

.login-card {
  width: 420px;
  padding: 40px;
  background: var(--bg-card, #ffffff);
  border-radius: 12px;
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.15);
}

.login-header {
  text-align: center;
  margin-bottom: 32px;
}

.logo {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  margin-bottom: 8px;
}

.logo-icon {
  font-size: 28px;
}

.logo-text {
  font-size: 24px;
  font-weight: 700;
  color: var(--text-primary, #1a1a2e);
}

.login-subtitle {
  font-size: 14px;
  color: var(--text-secondary, #888);
}

.login-options {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 24px;
}

.register-link {
  font-size: 13px;
  color: #2080f0;
  text-decoration: none;
}

.register-link:hover {
  text-decoration: underline;
}

.login-btn {
  font-size: 16px;
  letter-spacing: 4px;
}
</style>
