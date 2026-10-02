// e2e 公共夹具：起浏览器、登录、在真实会话里发请求、汇总结果。
//
// 三个回归套件（权限码/菜单删除守卫/角色追加守卫）都需要同一套东西，
// 原先每个脚本各抄一份 ~100 行，改一处要改三处。这里抽出来只留一份。

import { spawn } from 'node:child_process'
import { mkdirSync, rmSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { CDP, waitFor, sleep } from './cdp.mjs'

// 默认用 `localhost` 而非 `127.0.0.1`：vite 默认只绑 IPv6 的 `::1`，
// 写死 IPv4 会得到一个连不上的地址，而报错看起来像"服务没起"。
// `localhost` 两种栈都解析，具体落在哪边由 vite 的绑定决定。
const APP = process.env.E2E_APP || 'http://localhost:3000'
const CDP_PORT = Number(process.env.E2E_CDP_PORT || 9222)
const CDP_HTTP = `http://127.0.0.1:${CDP_PORT}`
const ARTIFACTS = process.env.E2E_ARTIFACTS || join(process.cwd(), 'e2e', '.artifacts')

/** 拉起一个 headless Chrome 并连上去；已有一个在跑就直接复用 */
async function launchBrowser() {
  const bin = process.env.CHROME_PATH ||
    '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome'
  const profile = join(ARTIFACTS, 'chrome-profile')

  // 先看有没有现成的（开发者可能已经开着带调试端口的 Chrome）
  try {
    const res = await fetch(CDP_HTTP + '/json/version')
    if (res.ok) return { version: await res.json(), proc: null }
  } catch { /* 没在跑，走下面的拉起流程 */ }

  rmSync(profile, { recursive: true, force: true })
  mkdirSync(profile, { recursive: true })
  const proc = spawn(bin, [
    '--headless=new',
    `--remote-debugging-port=${CDP_PORT}`,
    `--user-data-dir=${profile}`,
    '--no-first-run',
    '--no-default-browser-check',
    '--disable-gpu',
    '--hide-scrollbars',
    'about:blank',
  ], { stdio: 'ignore', detached: false })

  const deadline = Date.now() + 20000
  while (Date.now() < deadline) {
    try {
      const res = await fetch(CDP_HTTP + '/json/version')
      if (res.ok) return { version: await res.json(), proc }
    } catch { /* 还没起来 */ }
    await sleep(200)
  }
  proc.kill()
  throw new Error(`Chrome 未能在 20s 内就绪（${CDP_HTTP}）。可用 CHROME_PATH 指定浏览器。`)
}

export class Session {
  constructor(name) {
    this.name = name
    this.results = []
    this.consoleErrors = []
    this.badResponses = []
    this.downloads = join(ARTIFACTS, 'downloads')
    mkdirSync(join(ARTIFACTS, 'shots'), { recursive: true })
    mkdirSync(this.downloads, { recursive: true })
  }

  log(...a) { console.log(...a) }

  /** 记录一条断言。判据一律落在**可观测的事实**上，不落在"没报错"上。 */
  check(name, pass, detail = '') {
    this.results.push({ name, pass: !!pass, detail: String(detail ?? '') })
    console.log((pass ? 'PASS  ' : 'FAIL  ') + name +
      (detail ? '  ::  ' + detail : ''))
  }

  /** 等价于 check(!x)，但读起来更像断言 */
  checkAbsent(name, absent, detail = '') {
    this.check(name, !absent, detail)
  }

  async start() {
    const { version, proc } = await launchBrowser()
    this.proc = proc
    this.log('浏览器: ' + version.Browser)

    this.cdp = await CDP.connect(version.webSocketDebuggerUrl)

    // 每次跑都用全新浏览器上下文：localStorage 干净，下载目录可预期
    const { browserContextId } = await this.cdp.send('Target.createBrowserContext', {
      disposeOnDetach: false,
    })
    this.browserContextId = browserContextId
    await this.cdp.send('Browser.setDownloadBehavior', {
      behavior: 'allow',
      downloadPath: this.downloads,
      eventsEnabled: true,
      browserContextId,
    })

    const { targetId } = await this.cdp.send('Target.createTarget', {
      url: 'about:blank',
      browserContextId,
    })
    const { sessionId } = await this.cdp.send('Target.attachToTarget', {
      targetId, flatten: true,
    })
    this.sessionId = sessionId

    this.cdp.on('Runtime.consoleAPICalled', (p, sid) => {
      if (sid !== sessionId || p.type !== 'error') return
      this.consoleErrors.push(p.args.map((a) => a.value ?? a.description ?? a.type).join(' '))
    })
    this.cdp.on('Log.entryAdded', (p, sid) => {
      if (sid !== sessionId || p.entry.level !== 'error') return
      this.consoleErrors.push('[log] ' + p.entry.text)
    })
    this.cdp.on('Network.responseReceived', (p, sid) => {
      if (sid !== sessionId) return
      if (p.response.status >= 400) {
        this.badResponses.push(p.response.status + ' ' + p.response.url)
      }
    })

    await this.send('Page.enable')
    await this.send('Runtime.enable')
    await this.send('Network.enable')
    await this.send('Log.enable')
  }

  send(method, params = {}) {
    return this.cdp.send(method, params, this.sessionId)
  }

  /** 在页面里跑一段 async 函数体，返回其值 */
  async evalJs(body) {
    const r = await this.send('Runtime.evaluate', {
      expression: '(async () => { ' + body + ' })()',
      awaitPromise: true,
      returnByValue: true,
    })
    if (r.exceptionDetails) {
      const d = r.exceptionDetails
      throw new Error('页面内求值失败: ' + (d.exception?.description || d.text))
    }
    return r.result.value
  }

  async goto(path, settle = 700) {
    const loaded = new Promise((res) => this.cdp.on('Page.loadEventFired', () => res()))
    await this.send('Page.navigate', { url: APP + path })
    await loaded
    await sleep(settle)
  }

  async shot(name) {
    const r = await this.send('Page.captureScreenshot', { format: 'png' })
    const p = join(ARTIFACTS, 'shots', name + '.png')
    writeFileSync(p, Buffer.from(r.data, 'base64'))
    this.log('  截图 -> ' + p)
    return p
  }

  async setInput(placeholder, value) {
    const ok = await this.evalJs(
      'const el = [...document.querySelectorAll("input")].find(i => i.placeholder && i.placeholder.includes('
      + JSON.stringify(placeholder) + '));'
      + ' if (!el) return false;'
      + ' const s = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set;'
      + ' s.call(el, ' + JSON.stringify(value) + ');'
      + ' el.dispatchEvent(new Event("input", { bubbles: true }));'
      + ' el.dispatchEvent(new Event("change", { bubbles: true }));'
      + ' return true'
    )
    if (!ok) throw new Error('输入框未找到: ' + placeholder)
  }

  async clickByText(text) {
    return this.evalJs(
      'const b = [...document.querySelectorAll("button")].find(x => x.textContent.includes('
      + JSON.stringify(text) + '));'
      + ' if (!b) throw new Error("按钮未找到: ' + text + '");'
      + ' b.click(); return true'
    )
  }

  /**
   * 真登录一次，之后所有请求都带这个真实会话的令牌
   *
   * `expectPath` 是登录成功后**预期**落地的路径。受限令牌（管理员建号、
   * 待改密）会被路由守卫弹去 `/profile` 而不是首页——写死等 `"/"` 时，
   * 那种账号会在登录表单上一直等到超时，而真正的缺陷还没开始验证。
   *
   * 换身份前会**先登出当前会话**：路由守卫里带着令牌访问 `/login`
   * 会被弹回首页，于是第二次调用 `login()` 时连登录表单都等不到。
   */
  async login(user = process.env.E2E_USER || 'admin',
               pass = process.env.E2E_PASS || 'admin123',
               expectPath = '/') {
    // 登出走真实接口，让后端有机会记录/吊销，而不是只擦 localStorage。
    // 首次调用时页面还停在 about:blank，读 localStorage 会抛 SecurityError，
    // 那等价于"还没登录"，不该让整个套件挂在这里
    let existing = null
    try {
      existing = await this.currentToken()
    } catch {
      /* 还没落到应用页面上，按未登录处理 */
    }
    if (existing) {
      await this.apiAs(existing, 'POST', '/api/auth/logout').catch(() => {})
    }
    // 必须先导航到应用上才能碰 localStorage（about:blank 会拒绝访问）。
    // 而登出只吊销了后端会话，本地令牌还在——不清掉的话，
    // 路由守卫会带着旧令牌把 /login 弹回首页，登录表单永远等不到
    await this.goto('/login', 300)
    await this.evalJs('localStorage.clear(); return true')
    await this.goto('/login')
    await waitFor(() => this.evalJs('return !!document.querySelector("input")'),
      { label: '登录表单' })
    await this.setInput('请输入用户名', user)
    await this.setInput('请输入密码', pass)
    await this.clickByText('登 录')
    await waitFor(
      () => this.evalJs('return location.pathname === ' + JSON.stringify(expectPath)),
      { timeout: 20000, label: '登录后跳转到 ' + expectPath },
    )
    await sleep(900)
    return user
  }

  /**
   * 用浏览器会话里的真实令牌打后端接口。
   *
   * 走的是界面发的同一条路径（同源 + 同一份 localStorage 令牌），
   * 所以"界面能看见但接口 403"这类前后端不一致能被真实抓到。
   */
  async api(method, path, body) {
    return this.apiAs(await this.currentToken(), method, path, body)
  }

  /** 浏览器里当前这份令牌的原文（null 表示尚未登录） */
  currentToken() {
    return this.evalJs(
      'const raw = localStorage.getItem("axum_token");'
      + ' return raw ? JSON.parse(decodeURIComponent(atob(raw))).value : null'
    )
  }

  /**
   * 用指定令牌发请求，不动 localStorage。
   *
 * 跨身份场景（管理员建号 → 弱操作员尝试越权 → 目标用户验证会话）
   * 需要同时持有几份令牌；改写 localStorage 会把浏览器会话搅乱，
   * 所以这里只做"同源 + 指定 Authorization"，令牌从哪来由调用方决定。
   */
  apiAs(token, method, path, body) {
    return this.evalJs(
      'const res = await fetch(' + JSON.stringify(APP + path) + ', {'
      + ' method: ' + JSON.stringify(method) + ','
      + ' headers: { "Content-Type": "application/json", Authorization: "Bearer " + '
      + JSON.stringify(token || '') + ' },'
      + ' body: ' + (body ? JSON.stringify(JSON.stringify(body)) : 'undefined') + ' });'
      + ' let j = null; try { j = await res.json() } catch (e) {}'
      + ' return { status: res.status, body: j }'
    )
  }

  /**
   * 换一个身份拿一份令牌，不经过登录页、也不写 localStorage。
   *
   * 界面登录（`login`）留给人看的路径；这里给的是"脚本要以另一个人的身份
   * 调接口"的需求——两者验的是不同的东西，不能互相替代。
   */
  tokenFor(username, password) {
    return this.evalJs(
      'const res = await fetch(' + JSON.stringify(APP + '/api/auth/login') + ', {'
      + ' method: "POST", headers: { "Content-Type": "application/json" },'
      + ' body: JSON.stringify({ username: ' + JSON.stringify(username)
      + ', password: ' + JSON.stringify(password) + ' }) });'
      + ' let j = null; try { j = await res.json() } catch (e) {}'
      + ' if (res.status !== 200 || !j?.data?.token) {'
      // 提示词要整段带引号，否则拼出来的是裸代码而非字符串
      // （曾经写成 `new Error("user" + 登录失败: + status)`，页面直接报语法错）
      + '   throw new Error('
      + JSON.stringify(username + ' 登录失败: ')
      + '   + res.status + " " + JSON.stringify(j && j.message || "")) }'
      + ' return j.data.token'
    )
  }

  /**
   * 拿一份**可正常调用业务接口**的令牌（管理员建号 → 用户自助改密 → 再登录）
   *
   * v0.11.0 起，管理员建号会置 `must_change_password`，登录拿到的是**受限令牌**：
   * 后端只放行改密 / 登出 / `/me`，其余一律 403。于是"建号后直接拿令牌打接口"
   * 这条路径整体失效——拿到的 403 说的是"请先改密"，不是被测的那个权限边界，
   * 套件就会在跟产品无关的地方红一片。
   *
   * 这里走**真实产品流程**（自助改密）而不是直连数据库改标记：
   * 既不依赖测试库实现，也顺带把"管理员建号 → 用户改密 → 拿到正常会话"
   * 这条真实路径跑了一遍。改密会吊销该用户全部会话，所以最后必须重新登录。
   */
  async activatedToken(username, initialPassword, finalPassword) {
    const stale = await this.tokenFor(username, initialPassword)
    const changed = await this.apiAs(stale, 'PUT', '/api/auth/password', {
      old_password: initialPassword,
      new_password: finalPassword,
    })
    if (changed.status !== 200) {
      throw new Error(
        username + ' 激活失败（自助改密未成功）: ' + changed.status + ' '
        + JSON.stringify(changed.body?.message || ''),
      )
    }
    return this.tokenFor(username, finalPassword)
  }

  /** 清空 4xx/5xx 记录，用来把"预期内的 403"与意外错误区分开 */
  resetBadResponses() { this.badResponses.length = 0 }

  /**
   * 把**故意**造出来的失败从账上划掉：网络记录与控制台日志一起清。
   *
   * 为什么不能只靠 `checkNoUnexpectedHttp(label, expected)` 的白名单：
   * 一旦把某个状态码加进白名单，本套件里**任何**该状态码都会被当成预期——
   * 而"令牌意外过期""会话被误吊销"恰好都表现为 401，正是要抓的那类回归。
   * 就地划掉只豁免这一次，余下阶段该状态码仍是硬信号。
   *
   * `resetBadResponses` 刻意不清控制台：有的套件要在中途清网络噪声、
   * 但仍希望末尾检查整轮的控制台错误。
   */
  forgetDeliberateFailures() {
    this.badResponses.length = 0
    this.consoleErrors.length = 0
  }

  /** 除 `expected` 里列出的（如 "403 "）之外不该有别的 4xx/5xx */
  checkNoUnexpectedHttp(label, expected = []) {
    const unexpected = this.badResponses.filter((r) =>
      !expected.some((e) => r.startsWith(e)))
    this.check(label, unexpected.length === 0,
      unexpected.length ? unexpected.slice(0, 5).join(' | ') : '无')
  }

  /**
   * 控制台不该有错误。
   *
   * `expected` 与 `checkNoUnexpectedHttp` 共用同一份清单：套件**故意**触发的
   * 403/404，浏览器也会记一条 "Failed to load resource"。
   * 那种情况不是缺陷而是断言本身——若不排除，套件就必须在自己的
   * 核心断言和这条检查之间二选一，而"排除一切"会把真实错误一起吞掉。
   * 因此只放行清单里那几个状态码的网络日志，其他一律算错误。
   */
  checkNoConsoleErrors(expected = []) {
    const isExpectedNetworkNoise = (line) => {
      if (!line.includes('Failed to load resource')) return false
      const m = line.match(/status of (\d+)/)
      return !!m && expected.some((e) => e.trim().startsWith(m[1]))
    }
    const real = this.consoleErrors.filter(
      (e) => !e.includes('favicon') && !isExpectedNetworkNoise(e))
    this.check('无控制台错误', real.length === 0, real.slice(0, 5).join(' | '))
  }

  /** 打印汇总并返回失败条数；调用方据此决定退出码 */
  summary() {
    const failed = this.results.filter((r) => !r.pass)
    console.log('\n================ ' + this.name + ' 汇总 ================')
    console.log(`${this.results.length - failed.length}/${this.results.length} 通过`)
    for (const f of failed) console.log('  FAIL: ' + f.name + ' :: ' + f.detail)
    return failed.length
  }

  async stop() {
    try { this.cdp?.close() } catch { /* 忽略关闭异常 */ }
    // 只关自己拉起来的浏览器；复用的是开发者自己开的，不能动
    if (this.proc) { try { this.proc.kill() } catch { /* 已退出 */ } }
  }
}

export { APP, ARTIFACTS, waitFor, sleep }
