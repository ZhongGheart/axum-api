<template>
  <div class="page-container">
    <n-page-header
      title="个人中心"
      subtitle="查看本人账号信息并修改登录密码"
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
                placeholder="至少 8 位，含大写/小写/数字/符号中的两类"
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
import { computed, reactive, ref } from 'vue'
import type { FormInst, FormRules } from 'naive-ui'
import { useRouter } from 'vue-router'
import { useUserStore } from '@/stores/user'
import { authApi } from '@/api/auth'
import { showSuccess, showWarning } from '@/utils/message'
import { passwordIssues } from '@/utils/password'

const router = useRouter()
const userStore = useUserStore()

const roles = computed(() => userStore.userInfo?.roles ?? [])

const createdAt = computed(() => {
  const raw = userStore.userInfo?.created_at
  if (!raw) return '—'
  const d = new Date(raw)
  return Number.isNaN(d.getTime()) ? raw : d.toLocaleString()
})

const formRef = ref<FormInst | null>(null)
const submitting = ref(false)
const form = reactive({ oldPassword: '', newPassword: '', confirmPassword: '' })

const rules: FormRules = {
  oldPassword: [{ required: true, message: '请输入当前密码', trigger: ['input', 'blur'] }],
  newPassword: [
    { required: true, message: '请输入新密码', trigger: ['input', 'blur'] },
    {
      trigger: ['input', 'blur'],
      validator: (_rule, value: string) => {
        const problems = passwordIssues(value ?? '')
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
}

const canSubmit = computed(
  () =>
    !submitting.value &&
    form.oldPassword.length > 0 &&
    form.newPassword.length > 0 &&
    passwordIssues(form.newPassword).length === 0 &&
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
</style>
