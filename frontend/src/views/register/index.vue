<template>
  <div class="register-page">
    <div class="register-card">
      <!-- Logo & 标题 -->
      <div class="register-header">
        <div class="logo">
          <span class="logo-icon">⚡</span>
          <span class="logo-text">创建账号</span>
        </div>
        <p class="register-subtitle">注册 Axum Admin 账号</p>
      </div>

      <!-- 注册表单 -->
      <n-form
        ref="formRef"
        :model="formData"
        :rules="formRules"
        label-placement="left"
        label-width="auto"
        size="large"
        @submit.prevent="handleRegister"
      >
        <n-form-item label="用户名" path="username">
          <n-input
            v-model:value="formData.username"
            placeholder="3-50 个字符，字母或数字"
            :maxlength="50"
            clearable
          >
            <template #prefix>
              <n-icon><UserIcon /></n-icon>
            </template>
          </n-input>
        </n-form-item>

        <n-form-item label="邮箱" path="email">
          <n-input
            v-model:value="formData.email"
            placeholder="请输入邮箱地址"
            :maxlength="255"
            clearable
          >
            <template #prefix>
              <n-icon><MailIcon /></n-icon>
            </template>
          </n-input>
        </n-form-item>

        <n-form-item label="密码" path="password">
          <n-input
            v-model:value="formData.password"
            type="password"
            show-password-on="click"
            placeholder="至少 6 个字符"
            :maxlength="128"
            clearable
          >
            <template #prefix>
              <n-icon><LockIcon /></n-icon>
            </template>
          </n-input>
        </n-form-item>

        <n-form-item label="确认密码" path="confirmPassword">
          <n-input
            v-model:value="formData.confirmPassword"
            type="password"
            show-password-on="click"
            placeholder="请再次输入密码"
            :maxlength="128"
            clearable
          >
            <template #prefix>
              <n-icon><LockIcon /></n-icon>
            </template>
          </n-input>
        </n-form-item>

        <n-button
          type="primary"
          block
          size="large"
          :loading="submitting"
          attr-type="submit"
          class="register-btn"
        >
          注 册
        </n-button>

        <div class="login-link-wrap">
          <router-link to="/login">已有账号？去登录</router-link>
        </div>
      </n-form>
    </div>
  </div>
</template>

<script setup lang="ts">
/**
 * 注册页面
 *
 * - 表单校验规则对齐后端（用户名 3-50 字符，密码至少 6 位，邮箱需含 @）
 * - 密码确认校验
 * - 注册成功后自动跳转登录页
 */
import { ref } from 'vue'
import { useRouter } from 'vue-router'
import {
  PersonOutline as UserIcon,
  LockClosedOutline as LockIcon,
  MailOutline as MailIcon,
} from '@vicons/ionicons5'
import type { FormInst, FormRules } from 'naive-ui'
import { authApi } from '@/api/auth'
import { showSuccess } from '@/utils/message'
import { handleError } from '@/api/helper'

// ── 状态 ────────────────────────────────────────────────────────

const router = useRouter()
const formRef = ref<FormInst | null>(null)
const submitting = ref(false)

interface RegisterForm {
  username: string
  email: string
  password: string
  confirmPassword: string
}

const formData = ref<RegisterForm>({
  username: '',
  email: '',
  password: '',
  confirmPassword: '',
})

// ── 表单校验规则（对齐后端 service/auth.rs） ──────────────────────

const formRules: FormRules = {
  username: [
    { required: true, message: '请输入用户名', trigger: 'blur' },
    { min: 3, message: '用户名至少 3 个字符', trigger: 'blur' },
    { max: 50, message: '用户名不能超过 50 个字符', trigger: 'blur' },
  ],
  email: [
    { required: true, message: '请输入邮箱', trigger: 'blur' },
    { type: 'email', message: '邮箱格式不正确', trigger: 'blur' },
  ],
  password: [
    { required: true, message: '请输入密码', trigger: 'blur' },
    { min: 6, message: '密码至少 6 个字符', trigger: 'blur' },
  ],
  confirmPassword: [
    { required: true, message: '请再次输入密码', trigger: 'blur' },
    {
      validator: (_rule, value: string) => {
        if (value !== formData.value.password) {
          return new Error('两次输入的密码不一致')
        }
        return true
      },
      trigger: 'blur',
    },
  ],
}

// ── 注册提交 ────────────────────────────────────────────────────

async function handleRegister(): Promise<void> {
  try {
    await formRef.value?.validate()
    submitting.value = true

    // 口令经 HTTPS 明文提交，由服务端 Argon2 存储
    await authApi.register({
      username: formData.value.username,
      email: formData.value.email,
      password: formData.value.password,
    })

    showSuccess('注册成功，请登录')
    router.push('/login')
  } catch (error) {
    handleError(error)
  } finally {
    submitting.value = false
  }
}
</script>

<style scoped>
.register-page {
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: 100vh;
  background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
}

.register-card {
  width: 440px;
  padding: 40px;
  background: var(--bg-card, #ffffff);
  border-radius: 12px;
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.15);
}

.register-header {
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

.register-subtitle {
  font-size: 14px;
  color: var(--text-secondary, #888);
}

.login-link-wrap {
  text-align: center;
  margin-top: 20px;
}

.login-link-wrap a {
  font-size: 13px;
  color: #2080f0;
  text-decoration: none;
}

.login-link-wrap a:hover {
  text-decoration: underline;
}

.register-btn {
  font-size: 16px;
  letter-spacing: 4px;
}
</style>
