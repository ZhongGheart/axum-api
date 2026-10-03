/**
 * 会话失效的单点出口
 *
 * **为什么需要它**：令牌被服务端吊销（登出、改角色、改密、停用）之后，
 * 页面上的每个接口都会返回 401。此前 401 只有一个"弹一句'未授权，请重新登录'"
 * 的动作，于是用户看到的是：
 *
 * - 停在原页面（说了请重新登录，却不跳登录页）
 * - 令牌留在 localStorage，之后每次进页面都重演
 * - 守卫放行到 404 兜底，页面上写着 **"404 页面未找到"**
 *
 * 最后一条是方向性相反的诊断：会话没了 ≠ 页面不存在，按 404 去排查会
 * 去找根本不存在的路由。
 *
 * 本模块只做三件事：**记下原因**、**交回调执行跳转**、**同一原因不重复触发**。
 * 它不自己 import 路由或 store —— 那样会与 `router` 形成循环依赖
 * （`router` → `stores` → `api` → 本模块）。跳转由 `router/index.ts`
 * 在模块加载时注册进来，与 `utils/message.ts` 的 `registerGlobalApis` 同一套路。
 */

/** 会话结束原因在 sessionStorage 里的键 */
const REASON_KEY = 'axum_session_end_reason'

/** 同一原因在该时间窗内只处理一次（毫秒） */
const DEDUPE_WINDOW_MS = 1500

export interface SessionEnded {
  /** 后端给出的原话，不在前端另编一套通用文案 */
  message: string
}

type SessionEndedHandler = (reason: SessionEnded) => void

let handler: SessionEndedHandler | null = null
let lastMessage = ''
let lastAt = 0

/** 注册跳转回调（由 `router/index.ts` 在模块加载时调用） */
export function registerSessionEndedHandler(fn: SessionEndedHandler | null): void {
  handler = fn
  resetSessionEnded()
}

/**
 * 记下"会话为何结束"并触发跳转。
 *
 * @returns 本次是否真的处理了（同一原因短时间重复出现时返回 false）
 */
export function notifySessionEnded(message: string, now = Date.now()): boolean {
  if (message === lastMessage && now - lastAt < DEDUPE_WINDOW_MS) {
    return false
  }
  lastMessage = message
  lastAt = now

  // 先落盘再跳转：跳转可能失败或被合并，但登录页仍应能说出原因。
  // 用 sessionStorage 而非 localStorage —— 这是**本次标签页**的临时状态，
  // 关掉标签页就该消失，而"记住密码"的用户名必须留在 localStorage。
  try {
    sessionStorage.setItem(REASON_KEY, message)
  } catch {
    /* 隐私模式等场景下写不进去：不因此放弃跳转 */
  }

  handler?.({ message })
  return true
}

/** 登录页取走会话结束原因（取走即清除，避免下次登录仍显示） */
export function takeSessionEnded(): string | null {
  try {
    const v = sessionStorage.getItem(REASON_KEY)
    sessionStorage.removeItem(REASON_KEY)
    return v
  } catch {
    return null
  }
}

/**
 * 拼出登录页要显示的完整提示
 *
 * 后端那几种原因**本身就以"请重新登录"结尾**（`令牌已被注销，请重新登录`）。
 * 界面无条件再拼一次就成了"请重新登录，请重新登录"。
 * 抽成纯函数是为了能直接测——挂载整个登录页来验一句文案太重。
 */
export function buildSessionEndedMessage(reason: string | null): string {
  if (!reason) return ''
  return /请重新登录/.test(reason) ? reason : `${reason}，请重新登录`
}

/** 只清去重状态，供测试与登出时复位 */
export function resetSessionEnded(): void {
  lastMessage = ''
  lastAt = 0
}
