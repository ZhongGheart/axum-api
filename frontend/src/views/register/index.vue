<template>
  <AuthShell>
    <div class="register">
      <header class="register-header">
        <h1 class="register-title">创建账号</h1>
        <p class="register-subtitle">注册后可自行修改资料与密码</p>
      </header>

      <!-- 注册表单 -->
      <n-form
        ref="formRef"
        :model="formData"
        :rules="formRules"
        label-placement="top"
        size="large"
        @submit.prevent="handleRegister"
      >
        <n-form-item label="用户名" path="username">
          <n-input
            v-model:value="formData.username"
            :placeholder="USERNAME_PLACEHOLDER"
            :maxlength="USERNAME_MAX_LEN"
            clearable
            :input-props="{ autocomplete: 'username' }"
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
            :input-props="{ autocomplete: 'email' }"
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
            :placeholder="PASSWORD_PLACEHOLDER"
            :maxlength="PASSWORD_MAX_LEN"
            clearable
            :input-props="{ autocomplete: 'new-password' }"
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
            :maxlength="PASSWORD_MAX_LEN"
            clearable
            :input-props="{ autocomplete: 'new-password' }"
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
          attr-type="submit"
          :loading="submitting"
        >
          注册
        </n-button>

        <div class="register-footer">
          已有账号？<router-link to="/login">去登录</router-link>
        </div>
      </n-form>
    </div>
  </AuthShell>
</template>

<script setup lang="ts">
/**
 * 注册页面
 *
 * - 表单校验规则来自 `@/utils/accountRules`，与改密页、管理员建号同源
 *
 * v0.18.0 之前这里写着"对齐后端（…密码至少 6 位…）"，而那正是**没对齐**的一版：
 * 规则是 v0.10 时代手写的 `min: 6`，后端早已收紧为 8 位 + 两类字符。
 * 用户按提示填 `abcdefgh` 能过前端校验，填完整个表单才收到 400。
 *
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
import {
  emailRules,
  PASSWORD_PLACEHOLDER,
  passwordPolicyRules,
  USERNAME_MAX_LEN,
  USERNAME_PLACEHOLDER,
  usernameRules,
} from '@/utils/accountRules'
import { PASSWORD_MAX_LEN } from '@/utils/password'
import AuthShell from '@/components/common/AuthShell.vue'

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

// ── 表单校验规则 ────────────────────────────────────────────────
//
// 全部来自 `@/utils/accountRules`：口令部分委托 `utils/password.ts` 的
// `passwordIssues`，也就是后端契约测试 `password_policy_agrees_with_the_frontend_copy`
// 绑定的那个实现。这里不再自己写 min/max——v0.18.0 的缺陷正是"自己写了一份"。

const formRules: FormRules = {
  username: usernameRules,
  email: emailRules,
  password: passwordPolicyRules,
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
.register-header {
  margin-bottom: 24px;
}

.register-title {
  font-size: 20px;
  font-weight: 600;
  color: var(--text-primary);
}

.register-subtitle {
  margin-top: 4px;
  font-size: 13px;
  color: var(--text-secondary);
}

.register-footer {
  margin-top: 20px;
  font-size: 13px;
  color: var(--text-secondary);
  text-align: center;
}

.register-footer a {
  color: var(--primary-color);
}

.register-footer a:hover {
  text-decoration: underline;
}
</style>