/**
 * 全局消息提示 / 弹窗 / 通知工具
 *
 * 在 App.vue 中通过 `registerGlobalApis` 注入 Naive UI 的 API 实例，
 * 此后可在任意位置（包括 store、API 拦截器）调用消息弹窗。
 */

import type { MessageApi, DialogApi, NotificationApi } from 'naive-ui'

// ── 全局 API 引用（由 App.vue 注入） ────────────────────────────

let messageApi: MessageApi | null = null
let dialogApi: DialogApi | null = null
let notificationApi: NotificationApi | null = null

/** 注册全局 API（必须在 setup 中调用） */
export function registerGlobalApis(
  msg: MessageApi,
  dialog: DialogApi,
  notif: NotificationApi,
): void {
  messageApi = msg
  dialogApi = dialog
  notificationApi = notif
}

// ── 消息提示 ────────────────────────────────────────────────────

export function showMessage(
  content: string,
  type: 'info' | 'success' | 'warning' | 'error' = 'info',
  duration = 3000,
): void {
  messageApi?.[type]?.(content, { duration })
}

export function showSuccess(content: string): void {
  showMessage(content, 'success')
}

export function showWarning(content: string): void {
  showMessage(content, 'warning')
}

export function showError(content: string): void {
  showMessage(content, 'error', 5000)
}

// ── 对话框 ──────────────────────────────────────────────────────

export interface ConfirmOptions {
  title?: string
  content: string
  confirmText?: string
  cancelText?: string
}

export function showConfirm(options: ConfirmOptions): Promise<boolean> {
  return new Promise((resolve) => {
    dialogApi?.warning({
      title: options.title || '确认操作',
      content: options.content,
      positiveText: options.confirmText || '确定',
      negativeText: options.cancelText || '取消',
      onPositiveClick: () => resolve(true),
      onNegativeClick: () => resolve(false),
      onMaskClick: () => resolve(false),
    })
  })
}

// ── 通知 ────────────────────────────────────────────────────────

export function showNotification(
  title: string,
  content: string,
  type: 'info' | 'success' | 'warning' | 'error' = 'info',
  duration = 4000,
): void {
  notificationApi?.[type]?.({
    title,
    content,
    duration,
  })
}
