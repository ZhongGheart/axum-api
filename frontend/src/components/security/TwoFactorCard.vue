<template>
  <n-card title="两步验证" size="small">
    <template #header-extra>
      <n-tag v-if="status?.enabled" size="small" type="success" :bordered="false">
        已启用
      </n-tag>
      <n-tag v-else size="small" :bordered="false">未启用</n-tag>
    </template>

    <n-spin :show="loading">
      <!-- 已启用 -->
      <template v-if="status?.enabled">
        <n-descriptions :column="1" label-placement="left" bordered size="small">
          <n-descriptions-item label="生效时间">
            {{ formatTime(status.enabled_at) }}
          </n-descriptions-item>
          <n-descriptions-item label="剩余恢复码">
            <n-space align="center" :size="8">
              <n-tag
                size="small"
                :bordered="false"
                :type="status.recovery_codes_remaining > 0 ? 'success' : 'error'"
              >
                {{ status.recovery_codes_remaining }} 个可用
              </n-tag>
              <span v-if="status.recovery_codes_remaining === 0" class="hint danger">
                已用完。手机丢了就再也登不进来，请立刻重新生成一批。
              </span>
            </n-space>
          </n-descriptions-item>
        </n-descriptions>

        <n-space style="margin-top: 16px">
          <n-button :loading="busy" @click="openRegenerate">
            重新生成恢复码
          </n-button>
          <n-button type="error" secondary :loading="busy" @click="openDisable">
            关闭两步验证
          </n-button>
        </n-space>

        <div class="hint" style="margin-top: 8px">
          关闭需要出示当前密码：登录状态本身可能来自被盗的设备，
          只凭"已登录"就能关掉两步验证，这个功能就白做了。
        </div>
      </template>

      <!-- 未启用：入口 -->
      <template v-else>
        <p class="desc">
          开启后，登录除了密码还需要验证器 App 生成的 6 位动态码。
          账号密码泄露时，单凭密码不足以登录。
        </p>
        <n-button type="primary" :loading="busy" @click="startSetup">
          开始绑定
        </n-button>
      </template>
    </n-spin>

    <!-- 绑定第一步：扫码 / 输密钥 -->
    <n-modal
      v-model:show="setupVisible"
      title="绑定两步验证"
      preset="card"
      style="width: 440px"
      :mask-closable="false"
      :close-on-esc="!busy"
    >
      <n-steps :current="setupStep" size="small" class="steps">
        <n-step title="扫码" />
        <n-step title="验证" />
      </n-steps>

      <div v-if="setupStep === 1" class="setup-body">
        <n-alert type="info" :show-icon="true" class="setup-alert">
          用验证器 App（1Password、Google Authenticator 等）扫下面的二维码，
          然后回到这里输入它显示的 6 位码。
        </n-alert>

        <div class="qr-wrap">
          <!--
            二维码在前端渲染：后端返回的是 otpauth:// URI，
            出图只是把这串文字画成方块，没必要为此在服务端引图形依赖。
            扫不出来时下方还有手动输入的密钥兜底。
          -->
          <img v-if="qrDataUrl" :src="qrDataUrl" alt="两步验证绑定二维码" class="qr" />
          <n-spin v-else size="small" />
        </div>

        <n-collapse>
          <n-collapse-item title="扫不出来？手动输入密钥">
            <n-code :code="groupedSecret" :word-wrap="false" />
            <n-button
              quaternary
              size="small"
              style="margin-top: 8px"
              @click="copySecret"
            >
              复制密钥
            </n-button>
          </n-collapse-item>
        </n-collapse>

        <div class="hint">
          密钥只显示这一次。它在 App 侧确认成功前并未生效，
          中途放弃不会留下半配置状态。
        </div>
      </div>

      <div v-else class="setup-body">
        <n-alert type="success" :show-icon="true" class="setup-alert">
          App 已扫码成功，现在输入它显示的 6 位动态码完成绑定。
        </n-alert>
        <n-input-otp
          v-model:value="confirmCells"
          :length="6"
          :disabled="busy"
          size="large"
          @finish="submitEnable"
        />
        <div class="hint">码每 30 秒变一次。输错不影响密钥，重新输对的即可。</div>
      </div>

      <template #footer>
        <n-space justify="space-between">
          <n-button quaternary :disabled="busy" @click="closeSetup">
            {{ setupStep === 2 ? '返回上一步' : '取消' }}
          </n-button>
          <n-space>
            <n-button
              v-if="setupStep === 1"
              type="primary"
              :disabled="qrDataUrl === ''"
              @click="setupStep = 2"
            >
              下一步
            </n-button>
            <n-button
              v-else
              type="primary"
              :loading="busy"
              :disabled="!confirmCodeReady"
              @click="submitEnable"
            >
              确认启用
            </n-button>
          </n-space>
        </n-space>
      </template>
    </n-modal>

    <!-- 恢复码展示：一次性，关闭即不可再取 -->
    <n-modal
      v-model:show="codesVisible"
      title="保存你的恢复码"
      preset="card"
      style="width: 460px"
      :mask-closable="false"
      :close-on-esc="false"
    >
      <n-alert type="warning" :show-icon="true" class="setup-alert">
        这 {{ recoveryCodes.length }} 个码每个只能用一次，是手机不在手边时唯一的登录入口。
        <strong>关掉这个窗口后就再也看不到了</strong>，请先复制或抄下来。
      </n-alert>

      <div class="recovery-grid">
        <code v-for="(c, i) in recoveryCodes" :key="i" class="recovery-code">{{ c }}</code>
      </div>

      <n-space>
        <n-button size="small" @click="copyRecoveryCodes">复制全部</n-button>
        <n-button size="small" quaternary @click="downloadRecoveryCodes">
          下载为文本文件
        </n-button>
      </n-space>

      <template #footer>
        <n-button type="primary" @click="closeCodes">我已保存</n-button>
      </template>
    </n-modal>

    <!-- 关闭两步验证：要求出示密码 -->
    <n-modal
      v-model:show="disableVisible"
      title="关闭两步验证"
      preset="card"
      style="width: 400px"
    >
      <n-alert type="error" :show-icon="true" class="setup-alert">
        关闭后，你的账号将只靠密码保护。确认要继续吗？
      </n-alert>
      <n-input
        v-model:value="disablePassword"
        type="password"
        show-password-on="click"
        placeholder="请输入当前密码"
        @keyup.enter="submitDisable"
      />
      <template #footer>
        <n-space justify="end">
          <n-button :disabled="busy" @click="disableVisible = false">取消</n-button>
          <n-button
            type="error"
            :loading="busy"
            :disabled="disablePassword.length === 0"
            @click="submitDisable"
          >
            确认关闭
          </n-button>
        </n-space>
      </template>
    </n-modal>
  </n-card>
</template>

<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { authApi } from '@/api/auth'
import { showError, showSuccess } from '@/utils/message'
import { formatRecoveryCodes, groupSecret, joinOtp, renderQrDataUrl } from '@/utils/twoFactor'
import type { TwoFactorStatus } from '@/api/types/response'

/**
 * 两步验证绑定卡（v0.25.0）
 *
 * 流程刻意分三段而不是一步到位：
 * setup 拿密钥（**不生效**）→ 用户在 App 里扫码 → enable 交一个
 * App 生成的码回来才落库。少了中间那一步，"我扫了但 App 说密钥
 * 无效"就会变成一个用户自己解不开的死结。
 *
 * 恢复码明文只从后端取这一次，**关掉弹窗即不可再取**（库里只有
 * SHA-256 摘要）。所以弹窗刻意禁用遮罩点击与 ESC，并给出复制与
 * 下载两条出路——不给出路的话，用户多半直接点掉然后回不来。
 */

const status = ref<TwoFactorStatus | null>(null)
const loading = ref(false)
const busy = ref(false)

// ── 绑定流程 ────────────────────────────────────────────────────

const setupVisible = ref(false)
const setupStep = ref<1 | 2>(1)
const qrDataUrl = ref('')
const secret = ref('')

/** `n-input-otp` 的 value 是每格一个字符的数组，不是单个字符串 */
const confirmCells = ref<string[] | null>(null)

/** 只做展示分组，提交的密钥不受空格影响 */
const groupedSecret = ref('')

const confirmCode = computed(() => joinOtp(confirmCells.value))
const confirmCodeReady = computed(() => confirmCode.value.length === 6)

// ── 恢复码 ──────────────────────────────────────────────────────

const codesVisible = ref(false)
const recoveryCodes = ref<string[]>([])

// ── 关闭 ────────────────────────────────────────────────────────

const disableVisible = ref(false)
const disablePassword = ref('')

function formatTime(iso: string | null): string {
  if (!iso) return '—'
  const d = new Date(iso)
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString()
}

async function loadStatus() {
  if (loading.value) return
  loading.value = true
  try {
    status.value = (await authApi.twoFactorStatus()) as unknown as TwoFactorStatus
  } catch {
    // 错误提示已由响应拦截器统一弹出
  } finally {
    loading.value = false
  }
}

async function startSetup() {
  if (busy.value) return
  busy.value = true
  try {
    const setup = (await authApi.setupTwoFactor()) as unknown as {
      secret: string
      provisioning_uri: string
    }
    secret.value = setup.secret
    groupedSecret.value = groupSecret(setup.secret)
    // 二维码渲染失败不该中断绑定：密钥已经拿到，
    // 下方的手动输入入口足够撑住这个场景
    try {
      qrDataUrl.value = await renderQrDataUrl(setup.provisioning_uri)
    } catch {
      qrDataUrl.value = ''
      showError('二维码渲染失败，请手动输入密钥')
    }
    setupStep.value = 1
    confirmCells.value = null
    setupVisible.value = true
  } catch {
    // 错误提示已由响应拦截器统一弹出
  } finally {
    busy.value = false
  }
}

async function submitEnable() {
  if (busy.value || !confirmCodeReady.value) return
  busy.value = true
  try {
    const resp = (await authApi.enableTwoFactor(confirmCode.value)) as unknown as {
      recovery_codes: string[]
    }
    // 先弹恢复码再关绑定弹窗：顺序反了会出现
    // "两个弹窗都没了、恢复码没看见"的空档
    showRecoveryCodes(resp.recovery_codes)
    setupVisible.value = false
    confirmCells.value = null
    await loadStatus()
  } catch {
    // 错误提示已由响应拦截器统一弹出。密钥仍在待确认槽位里，可直接重试
  } finally {
    busy.value = false
  }
}

function closeSetup() {
  if (setupStep.value === 2) {
    setupStep.value = 1
    confirmCells.value = null
    return
  }
  setupVisible.value = false
  // 丢弃本地密钥显示。服务端那个待确认密钥 15 分钟后自然过期，
  // 主动清理反而要多一次往返。
  secret.value = ''
  qrDataUrl.value = ''
}

function showRecoveryCodes(codes: string[]) {
  recoveryCodes.value = codes
  codesVisible.value = true
}

function closeCodes() {
  codesVisible.value = false
  recoveryCodes.value = []
}

async function openRegenerate() {
  if (busy.value) return
  // 先确认再动手：重新生成会让旧的一批立刻作废，
  // 用户手边可能还留着旧码
  if (!window.confirm('重新生成后，旧的一批恢复码会立即失效。确认继续？')) return
  busy.value = true
  try {
    const resp = (await authApi.regenerateRecoveryCodes()) as unknown as {
      recovery_codes: string[]
    }
    showRecoveryCodes(resp.recovery_codes)
    await loadStatus()
  } catch {
    // 错误提示已由响应拦截器统一弹出
  } finally {
    busy.value = false
  }
}

function openDisable() {
  disablePassword.value = ''
  disableVisible.value = true
}

async function submitDisable() {
  if (busy.value || disablePassword.value.length === 0) return
  busy.value = true
  try {
    await authApi.disableTwoFactor(disablePassword.value)
    disableVisible.value = false
    showSuccess('两步验证已关闭')
    await loadStatus()
  } catch {
    // 错误提示已由响应拦截器统一弹出
  } finally {
    busy.value = false
  }
}

// ── 复制 / 下载 ────────────────────────────────────────────────

async function copyToClipboard(text: string, what: string) {
  try {
    await navigator.clipboard.writeText(text)
    showSuccess(`${what}已复制`)
  } catch {
    showError(`复制失败，请手动选中${what}`)
  }
}

function copySecret() {
  copyToClipboard(secret.value, '密钥')
}

function copyRecoveryCodes() {
  copyToClipboard(formatRecoveryCodes(recoveryCodes.value), '恢复码')
}

/**
 * 下载恢复码为文本文件
 *
 * 比"复制到剪贴板"更可靠的一手：剪贴板会被下一次 Ctrl+C 冲掉，
 * 文件却可以拖进密码管理器或存进保险箱。
 */
function downloadRecoveryCodes() {
  const body = [
    'Axum Admin 两步验证恢复码',
    `生成时间：${new Date().toLocaleString()}`,
    '',
    ...recoveryCodes.value,
    '',
    '每个码只能用一次。全部用完后请在个人中心重新生成一批。',
  ].join('\n')
  const url = URL.createObjectURL(new Blob([body], { type: 'text/plain;charset=utf-8' }))
  const a = document.createElement('a')
  a.href = url
  a.download = 'axum-api-2fa-recovery-codes.txt'
  a.click()
  // 不 revoke 会让这个 blob 一直挂在内存里直到页面刷新
  URL.revokeObjectURL(url)
}

onMounted(loadStatus)
</script>

<style scoped>
.desc {
  margin: 0 0 12px;
  font-size: 13px;
  color: var(--text-secondary);
  line-height: 1.6;
}

.hint {
  margin-top: 8px;
  font-size: 12px;
  color: var(--text-tertiary);
  line-height: 1.6;
}

.hint.danger {
  color: var(--danger-color);
}

.steps {
  margin-bottom: 16px;
}

.setup-body {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.setup-alert {
  margin-bottom: 4px;
}

.qr-wrap {
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: 200px;
}

.qr {
  width: 200px;
  height: 200px;
  /*
   * 二维码必须固定白底，不能跟随主题反色：
   * 深色底上部分验证器 App 会直接扫不出来，而这正是
   * "页面看着挺漂亮、绑定卡死在第一步"的典型成因。
   */
  background: #ffffff;
  border-radius: var(--radius-md);
  padding: 8px;
}

.recovery-grid {
  display: grid;
  grid-template-columns: repeat(2, 1fr);
  gap: 8px;
  margin: 16px 0;
}

.recovery-code {
  padding: 8px 10px;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 13px;
  letter-spacing: 0;
  text-align: center;
  background: var(--bg-hover);
  border-radius: var(--radius-sm);
  /* 让用户一次拖中一个码，而不是逐字选中 */
  user-select: all;
}
</style>
