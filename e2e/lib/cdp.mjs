// 最小 CDP 驱动：Node 22 内置 WebSocket，无需安装 Playwright / Puppeteer。
//
// 为什么不引第三方库：整个 e2e 只需要"导航 / 求值 / 截图 / 监听网络"四件事，
// 为此拉进 ~300MB 的 Playwright 不划算，而 Node 22 自带 WebSocket 与 fetch，
// 直连 CDP 足够。代价是要自己处理 target/session 分离，见 `attach`。

export class CDP {
  #ws
  #id = 0
  #pending = new Map()
  #listeners = new Map()

  constructor(ws) {
    this.#ws = ws
    ws.addEventListener('message', (ev) => {
      const msg = JSON.parse(ev.data)
      if (msg.id !== undefined) {
        const p = this.#pending.get(msg.id)
        if (!p) return
        this.#pending.delete(msg.id)
        if (msg.error) p.reject(new Error(p.method + ': ' + msg.error.message))
        else p.resolve(msg.result)
        return
      }
      for (const fn of this.#listeners.get(msg.method) || []) fn(msg.params, msg.sessionId)
    })
  }

  static async connect(url) {
    const ws = new WebSocket(url)
    await new Promise((res, rej) => {
      ws.addEventListener('open', res, { once: true })
      ws.addEventListener('error', rej, { once: true })
    })
    return new CDP(ws)
  }

  send(method, params = {}, sessionId) {
    const id = ++this.#id
    const payload = { id, method, params }
    if (sessionId) payload.sessionId = sessionId
    this.#ws.send(JSON.stringify(payload))
    return new Promise((resolve, reject) => {
      this.#pending.set(id, { resolve, reject, method })
    })
  }

  on(method, fn) {
    if (!this.#listeners.has(method)) this.#listeners.set(method, [])
    this.#listeners.get(method).push(fn)
  }

  close() {
    this.#ws.close()
  }
}

/**
 * 超时即抛错的等待器。
 *
 * 曾经写成"超时只记一条失败然后继续跑"，导致后续断言全建立在
 * 一个根本没出现的页面上，整轮回归的结论都是假的。
 * 这里的每一处等待都必须真的等到，否则就是假绿。
 */
export async function waitFor(fn, opts = {}) {
  const timeout = opts.timeout || 15000
  const interval = opts.interval || 200
  const deadline = Date.now() + timeout
  let last
  while (Date.now() < deadline) {
    last = await fn()
    if (last) return last
    await new Promise((r) => setTimeout(r, interval))
  }
  throw new Error(
    'waitFor 超时（' + (opts.timeout || timeout) + 'ms）: ' + (opts.label || '(未命名)') +
    '，最后一次取值: ' + JSON.stringify(last)
  )
}

export const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
