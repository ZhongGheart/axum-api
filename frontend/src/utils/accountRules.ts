/**
 * 账号表单的共享校验规则（创建账号 / 登录 三处共用）
 *
 * ## 为什么要有这个文件
 *
 * v0.11.0 把口令策略从"至少 6 位"收紧为"至少 8 位 + 两类字符"，并新建了
 * `utils/password.ts` 与一条把它绑到后端的契约测试。**但那次只迁移了改密页**：
 * 注册页与管理员建号对话框仍留着 v0.10 时代写下的 `min: 6`，整整七版没人碰。
 *
 * 实测（真实 HTTP）：用户在注册页按提示填 `abcdefgh`（前端放行）→ 提交后
 * 才收到 `400 密码复杂度不足`。填完整个表单才被告知规则是什么。
 *
 * 契约测试只把 `password.ts` 绑到了后端，**没有任何东西把"页面"绑到
 * `password.ts`** ——这正是它能漂移七版的原因。本文件加上第 6 步的
 * `account_forms_use_the_shared_validator` 就是补这个洞。
 *
 * ## 唯一裁决方仍是后端
 *
 * 与 `utils/password.ts` 同理：这里的规则只为**即时反馈**，不是安全边界。
 * 后端 `src/utils/validation.rs` 说了算，两侧靠 `USERNAME_POLICY_CASES` 对账。
 */

import type { FormItemRule } from 'naive-ui'
import { PASSWORD_MIN_LEN, passwordIssues } from './password'

/** 用户名最小长度（按**字符**计，须与后端 `USERNAME_MIN_LEN` 一致） */
export const USERNAME_MIN_LEN = 3
/** 用户名最大长度（按字符计，与 `users.username` 的 varchar(50) 同单位） */
export const USERNAME_MAX_LEN = 50
/** 登录标识符最大长度：与 `users.email` 的 varchar(255) 对齐 */
export const IDENTIFIER_MAX_LEN = 255

/**
 * 用户名字符集：Unicode 字母/数字 + 下划线 + 连字符
 *
 * **必须用 `\p{L}\p{N}` 而不是 `[a-zA-Z0-9]`**。后端是 Rust 的
 * `char::is_alphanumeric()`，那是 Unicode 感知的——实测 `用户名` 能注册成功。
 * 若这里写成 ASCII 白名单，中文用户名会被前端拦下而后端放行，
 * 漂移方向只是反过来，缺陷照旧。
 *
 * （Rust 的 `is_alphabetic` 还含 Other_Alphabetic，JS 的 `\p{L}` 不含；
 * 两者在拉丁/CJK/希腊/西里尔等实际用到的字母上完全一致，差异只落在
 * 少数组合记号上，不值得为它引入依赖。）
 */
const USERNAME_PATTERN = /^[\p{L}\p{N}_-]+$/u

/**
 * 用户名归一（trim + 小写）——必须与后端 `normalize_username` 同形
 *
 * **校验的对象必须是归一后的值**，不能是输入框里的原文。
 * 后端归一在校验之前，于是 `  alice  ` 在后端是合法的 `alice`；
 * 若这里按原文判，空格既不在字符集里、长度也超了，前端会红着拦下一个
 * 后端明明接受的输入。漂移方向与 `accountRules.ts` 顶部记的那些完全一样：
 * 前端比后端严，用户填完整个表单才知道规则。
 */
export function normalizeUsername(username: string): string {
  return username.trim().toLowerCase()
}

/** 用户名的问题列表；空数组表示通过 */
export function usernameIssues(username: string): string[] {
  const normalized = normalizeUsername(username)
  const problems: string[] = []
  const len = [...normalized].length
  if (len < USERNAME_MIN_LEN) {
    problems.push(`至少 ${USERNAME_MIN_LEN} 个字符`)
  }
  if (len > USERNAME_MAX_LEN) {
    problems.push(`最多 ${USERNAME_MAX_LEN} 个字符`)
  }
  if (len > 0 && !USERNAME_PATTERN.test(normalized)) {
    problems.push('只能包含字母、数字、下划线和连字符')
  }
  return problems
}

/** 邮箱归一（trim + 小写）——与后端 `normalize_email` 同形，理由同 `normalizeUsername` */
export function normalizeEmail(email: string): string {
  return email.trim().toLowerCase()
}

/** 邮箱的问题列表；空数组表示通过 */
export function emailIssues(email: string): string[] {
  const problems: string[] = []
  const normalized = normalizeEmail(email)
  if (!normalized.includes('@') || !normalized.includes('.')) {
    problems.push('格式需包含 @ 与 .')
  }
  if ([...normalized].length > 255) {
    problems.push('最长 255 个字符')
  }
  return problems
}

/**
 * 把问题列表包装成 naive-ui 的 validator
 *
 * naive-ui 的 validator 返回 `Error` 对象而不是字符串，直接返回字符串会被
 * 当成"校验通过"。改密页此前是内联写的，这里抽出来避免第三份实现。
 */
function toValidator(issues: (value: string) => string[]) {
  return (_rule: unknown, value: string | null | undefined) => {
    const problems = issues(value ?? '')
    return problems.length === 0 ? true : new Error(problems.join('；'))
  }
}

// 刻意不加 `as const`：naive-ui 的 FormItemRule.trigger 类型是 `string | string[]`，
// 只读元组赋不进去（typecheck 会报）。要窄类型就在下面显式标注。
const TRIGGER: string[] = ['input', 'blur']

/** 用户名字段规则：创建账号与管理员建号共用 */
export const usernameRules: FormItemRule[] = [
  { required: true, message: '请输入用户名', trigger: TRIGGER },
  { trigger: TRIGGER, validator: toValidator(usernameIssues) },
]

/** 邮箱字段规则 */
export const emailRules: FormItemRule[] = [
  { required: true, message: '请输入邮箱', trigger: TRIGGER },
  { trigger: TRIGGER, validator: toValidator(emailIssues) },
]

/**
 * 口令字段规则：**设置**口令时套用完整策略
 *
 * 注册页与管理员建号对话框都用它。v0.18.0 之前这两处写的是 `min: 6`，
 * 那是 v0.10 的策略化石——现在委托给 `passwordIssues`，与改密页同源。
 */
export const passwordPolicyRules: FormItemRule[] = [
  { required: true, message: '请输入密码', trigger: TRIGGER },
  { trigger: TRIGGER, validator: toValidator(passwordIssues) },
]

/**
 * 口令字段规则：**登录**时只要求非空，刻意不套用策略
 *
 * 后端有一条约束测试 `password_policy_is_not_applied_to_login_verification`
 * 明说：策略一旦挂到登录路径，抬高门槛的当天所有存量弱口令用户会被锁在门外。
 * 登录页原来那条 `min: 6` 虽然方向上是宽松的、不至于锁死人，但它是条
 * **没有任何依据**的规则——后端登录接受任意长度。这里与后端对齐：
 * 只判空，复杂度交给服务端。
 */
export const loginPasswordRules: FormItemRule[] = [
  { required: true, message: '请输入密码', trigger: TRIGGER },
]

/**
 * 登录标识符规则：只判空 + 长度上限，**刻意不校验字符集**
 *
 * 这个字段收的是"用户名**或邮箱**"，而后端 `find_by_username_or_email`
 * 是一条裸的 `WHERE username = $1 OR email = $1`，没有做任何格式校验。
 *
 * 所以这里**不能套 `usernameRules`**：邮箱含 `@`，会被字符集规则拒掉——
 * 用邮箱登录的人会在前端被自己人拦住，而后端明明放行。这不是理论风险，
 * 实测 `admin@example.com` 能正常登录。
 *
 * 长度上限取 255 而不是用户名的 50：`users.email` 是 `varchar(255)`，
 * 后端注册接口也接受长到这个上限的邮箱。原登录页写死 `max: 50`，
 * 于是**持有长邮箱的合法用户连自己的邮箱都输不进去**（实测 73 字符的邮箱
 * 注册与登录都成功，但输入框根本敲不进第 51 个字符之后的内容）。
 */
export const loginIdentifierRules: FormItemRule[] = [
  { required: true, message: '请输入用户名或邮箱', trigger: TRIGGER },
  {
    max: IDENTIFIER_MAX_LEN,
    message: `不能超过 ${IDENTIFIER_MAX_LEN} 个字符`,
    trigger: TRIGGER,
  },
]

/** 用户名输入框的 placeholder 文案（与上面的规则同源，别再各写各的） */
export const USERNAME_PLACEHOLDER = `${USERNAME_MIN_LEN}-${USERNAME_MAX_LEN} 个字符，字母、数字、下划线或连字符`

/** 口令输入框在**设置**口令时的 placeholder */
export const PASSWORD_PLACEHOLDER = `至少 ${PASSWORD_MIN_LEN} 位，含大写、小写、数字、符号中的两类`

/**
 * 跨语言契约样例（用户名）
 *
 * 格式被后端集成测试 `username_policy_agrees_with_the_frontend_rules` 逐行解析
 * （`{ name: '...', ok: true|false }`），用 Rust 的 `validate_username` 跑同一批
 * 取值并比对结论。**改动格式会让那条测试解析失败**——这正是它该有的行为。
 */
export const USERNAME_POLICY_CASES: ReadonlyArray<{ name: string; ok: boolean }> = [
  { name: 'alice', ok: true },
  { name: 'alice_01', ok: true },
  { name: 'alice-01', ok: true },
  { name: '用户名', ok: true },
  { name: 'Ωμέγα', ok: true },
  // **这一条是按字节/按字符的分水岭**：17 个汉字 = 51 字节、17 个字符。
  // 后端若改回 `username.len()`（字节），它会因为"超长"被判失败，
  // 而前端按字符算认为合法 —— 两侧结论就此分叉。
  // 少这条样例，上面那个回归可以让契约测试一路绿着通过。
  //
  // **必须写字面量，不能写 `'一'.repeat(17)`**：Rust 侧是按文本解析这张表的
  // （找不到下一个单引号就截断），`.repeat()` 会被读成样例 `一`——
  // 1 个字符，连长度下限都过不了，测试立刻红。第一次就是这么写错的。
  { name: '一一一一一一一一一一一一一一一一一', ok: true },
  // 星平面**字母**：30 个码点（合法）、60 个 UTF-16 码元（超上限）。
  // 后端 `chars().count()` 数码点，所以前端也必须数码点——
  // 若前端哪天真改成了 `.length`，这条会让契约测试立刻红。
  // （两侧对 U+1D49C 的判定是一致的：Rust `is_alphabetic()` 为 true，
  //   JS 的 `\p{L}` 也匹配，所以它是合法用户名，不是取巧的取值。）
  { name: '𝒜𝒜𝒜𝒜𝒜𝒜𝒜𝒜𝒜𝒜𝒜𝒜𝒜𝒜𝒜𝒜𝒜𝒜𝒜𝒜𝒜𝒜𝒜𝒜𝒜𝒜𝒜𝒜𝒜𝒜', ok: true },
  // 下面三条是 v0.19.0 的分水岭：**两端空白不算问题、大小写算同一个**，
  // 因为后端归一在校验之前。后端若改成先校验后归一，这三条会立刻红。
  { name: '  alice  ', ok: true },
  { name: 'ALICE', ok: true },
  // **只能写空格，不能写 '\t' / '\r' 这类转义**：后端契约测试是按**文本**
  // 解析这张表的（找下一个单引号截断），它会拿到字面的反斜杠 t，
  // 而不是制表符——于是 JS 侧 trim 掉的是空白、Rust 侧看到的是 `\tadmin\r`
  // 这串反斜杠，两侧结论必然相反，而报错只会说"不一致"。
  // 空白归一由上面两条空格样例覆盖，足够了。
  { name: '   ', ok: false },
  { name: 'ab', ok: false },
  { name: 'alice space', ok: false },
  { name: 'alice@host', ok: false },
  { name: 'alice.dot', ok: false },
]
