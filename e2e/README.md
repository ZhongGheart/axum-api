# e2e 回归

在**真实 Chrome** 里跑端到端回归，验的是"界面 + 接口 + 数据库"三者合起来
是否自洽。这层补的是单元测试和 `tests/api_integration.rs` 天然看不见的东西：

- 按钮看得见但一提交就 403（前后端权限码对不上）
- 路由能进但页面空白（前端组件/权限码缺失）
- 跨身份场景：管理员建号 → 弱操作员尝试越权 → 目标用户验证会话

## 为什么不引 Playwright

整个 e2e 只需要"导航 / 求值 / 截图 / 监听网络"四件事。为此拉进 ~300MB 的
Playwright 不划算——Node 22 自带 `WebSocket` 与 `fetch`，直连 CDP 足够。
代价是要自己处理 target/session 分离，见 `lib/cdp.mjs`。
因此 `lib/cdp.mjs` 是零依赖的，`node e2e/run.mjs` 开箱即跑，不需要 `npm install`。

## 前置

```bash
# 1) 依赖（Postgres + Redis），已在跑可跳过
scripts/test_env.sh start

# 2) 后端：必须**连测试库**，跑在 8080
export DATABASE_URL="postgres://postgres@127.0.0.1:55432/axum_api_test"
export REDIS_URL="redis://127.0.0.1:56379"
export JWT_SECRET="integration-test-secret-value-0123456789"
export SERVER_PORT=8080

# 限流必须放宽，否则套件与探针的请求量会把 IP 桶打满
# （默认 IP 100 次/分，三个套件连跑必然超；超了会表现为
#  "无控制台错误" 和 "无意外 4xx/5xx" 两条假红）
export RATE_LIMIT_IP_MAX=100000
export RATE_LIMIT_USER_MAX=100000
cargo run

# 3) 前端：dev server 在 3000（vite.config.ts 里固定）
cd frontend && npm run dev
```

后端必须**连测试库**：这些套件会建临时角色和账号，跑在开发库上会留垃圾数据。

## 跑

```bash
node e2e/run.mjs                    # 全部套件
node e2e/run.mjs role-assignment    # 只跑名字含该关键字的套件
```

退出码非 0 即有套件失败。套件之间是独立进程，一个崩了不影响其余的结论。

授权探针是**独立**脚本（它自己按 OpenAPI 发现入口，不必先跑套件）：

```bash
node e2e/probe-write-guards.mjs
```

⚠️ 探针与套件都必须在放宽限流的后端上跑。否则一轮下来 IP 桶会被打满，
后续套件里"无控制台错误""无意外 4xx/5xx"这两条会以 429 的形式假红——
那是限流在正常工作，不是产品缺陷，但排查起来很浪费时间。

## 配置（都有默认值）

| 变量 | 默认 | 说明 |
| --- | --- | --- |
| `E2E_APP` | `http://localhost:3000` | 前端地址（用 `localhost` 而非 `127.0.0.1`：vite 默认只绑 IPv6） |
| `E2E_CDP_PORT` | `9222` | Chrome 调试端口 |
| `E2E_ARTIFACTS` | `e2e/.artifacts` | 截图与下载落地目录 |
| `CHROME_PATH` | Chrome 默认安装路径 | 非默认位置时指定 |
| `E2E_USER` / `E2E_PASS` | `admin` / `admin123` | 登录用账号 |

浏览器会自己拉起 headless Chrome。若 `9222` 上已有 Chrome（你自己开的调试实例）
就直接复用，且**不会去关它**——只关自己拉起来的那个。

## 现有套件

| 套件 | 覆盖 |
| --- | --- |
| `permission-and-monitor.mjs` | 权限码入口与监控页导出（v0.7.0） |
| `menu-delete-guard.mjs` | 菜单删除的授权下界，含子树级联（v0.8.0） |
| `role-assignment-guard.mjs` | 角色追加的目标下界、会话吊销、幂等、404（v0.9.0） |
| `v010-ui-truth.mjs` | 新增筛选控件与分页必须真的生效（v0.10.0） |
| `v011-audit-and-password.mjs` | 登录审计在日志页看得见、受限令牌被界面拦住、界面表单改密（v0.11.0） |
| `v013-audit-change-summary.mjs` | 「变更摘要」列在界面与导出 xlsx 里都读得到、口令不入库（v0.13.0） |
| `v014-retention-honesty.mjs` | 日志页如实说明保留天数与现存最早一条、筛到已清理区间时提示（v0.14.0） |

## 加新套件

放进 `e2e/suites/`，从 `lib/harness.mjs` 拿 `Session`：

```js
import { Session } from '../lib/harness.mjs'
const s = new Session('我的套件')
await s.start()
await s.login()                       // 界面真实登录
const r = await s.api('GET', '/api/admin/users')   // 用浏览器里这份令牌打接口
s.check('用户列表可拉取', r.status === 200, 'status=' + r.status)
await s.shot('my-suite')              // 截图落到 .artifacts/shots/
const failed = s.summary()
await s.stop()
process.exit(failed ? 1 : 0)
```

跨身份时用 `s.tokenFor(user, pass)` 拿令牌、`s.apiAs(tok, ...)` 指定身份发请求，
**不要改写 localStorage**——那会把浏览器里 admin 的会话搅乱。

写断言前先确认**前置数据由套件自己造**。`v010-ui-truth.mjs` 曾断言"能翻到第二页"，
而它能不能成立取决于库里是否碰巧已有 10 个以上角色——干净库里只有
`admin`/`user` 两个，第二页压根不存在。那一版能过，只是因为跑它之前刚跑过
集成测试、库被污染了。换个干净库就红，而红的原因与被测的界面毫无关系。
现在该套件自己按当前总数补足到 `page_size + 1`，跑完再逐个删掉并核对残留。

同理要注意**下载文件名的去重方式**：前端把导出名写死成「操作日志.xlsx」，
Chrome 遇到同名文件是**覆盖**而不是加「(1)」后缀。所以"目录里多出一个新文件"
这种判据永远不成立（上一次跑的残留会把判据永久钉死），要改看修改时间。

写断言的两条纪律：

1. 判据落在**可观测事实**上，别落在状态码上。`200` 也可能什么都没写。
   每条"拒绝侧"断言配一条数据侧断言。
2. 每条"拒绝侧"配一条"放行侧"。只测拒绝的话，一个"把功能整个禁掉"的实现
   也能全绿——套件 3 的 [3]、[5] 就是干这个的。
