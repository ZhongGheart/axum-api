/**
 * 口令策略（前端副本）
 *
 * **后端 `src/utils/validation.rs` 的 `validate_password_with` 才是唯一裁决方。**
 * 这里重复实现只为即时反馈，不是安全边界。
 *
 * 重复实现必然有漂移风险，因此两侧共用一份判定样例：
 * `PASSWORD_POLICY_CASES`。后端有一条集成测试读取本文件，
 * 用 Rust 的 `validate_password` 跑同一批口令并比对结论——
 * 规则一旦在任一侧被改动而另一侧没跟上，那条测试立刻变红。
 *
 * ## v0.22.0 起策略是运行时取值，不再是常量
 *
 * 此前这里的 8 位 / 两类字符是写死的常量，而管理员能在「系统参数」页
 * 改它们——那会造成一个很坏的局面：**界面提示的规则与后端实际执行的规则
 * 分叉**，用户按界面提示填一个合规口令，提交后被拒，而报错说的是另一件事。
 *
 * 所以规则判定必须吃一份**传入的策略**：登录 / 注册 / 改密三处先取
 * `GET /api/settings/password-policy`，再把拿到的策略喂给 `passwordIssues`。
 * 下面的常量退化为「取不到策略时的回落值」，它们必须与后端
 * `PasswordPolicy::default()` 逐字一致（由
 * `settings_defaults_match_the_hardcoded_constants` 与
 * `password_length_constants_agree_with_the_backend` 双向钉住）。
 */

/** 口令最小长度（须与后端 `PASSWORD_MIN_LEN` 一致） */
export const PASSWORD_MIN_LEN = 8

/** 口令最大长度（须与后端 `PASSWORD_MAX_LEN` 一致） */
export const PASSWORD_MAX_LEN = 128

/** 口令至少要命中的字符类别数（须与后端 `PASSWORD_MIN_CHAR_CLASSES` 一致） */
export const PASSWORD_MIN_CHAR_CLASSES = 2

/**
 * 口令策略（与后端 `PasswordPolicy` 的公开字段同名）
 *
 * 只含**前端要判定**的四条。`expiry_days` 不在这里：
 * 它决定的是「登录后要不要下发受限令牌」，而受限令牌由后端在下发时判定，
 * 前端在改密页无从也无需参与——把它塞进来只会诱使人写一段假的过期判定。
 */
export interface PasswordPolicy {
  min_length: number
  max_length: number
  min_char_classes: number
  require_mixed_case: boolean
}

/**
 * 回落策略：只在**取不到服务端策略**时使用
 *
 * 刻意与后端 `PasswordPolicy::default()` 一致，于是「提示不准」
 * 这个降级后果最多让用户多改一次口令，不会让一个合规口令被拒。
 */
export const DEFAULT_PASSWORD_POLICY: PasswordPolicy = {
  min_length: PASSWORD_MIN_LEN,
  max_length: PASSWORD_MAX_LEN,
  min_char_classes: PASSWORD_MIN_CHAR_CLASSES,
  require_mixed_case: false,
}

/** 五类字符的可读名（顺序即判定顺序，与后端一致） */
export const CHAR_CLASS_NAMES: readonly string[] = [
  '大写字母',
  '小写字母',
  '数字',
  '符号',
  '非 ASCII 字母',
]

/** 中文数字；只到 5，与 `min_char_classes` 的取值范围一致 */
const CN_NUMERALS = ['', '一', '两', '三', '四', '五']

/**
 * 把「至少 N 类字符」渲染成一句中文
 *
 * **判定失败的原因（`passwordIssues`）与界面上的要求提示
 * （`describePasswordPolicy`）必须调用同一个函数**。
 * 两处各写一份时，管理员改一次参数就要改两处文案，
 * 而漏改的那一处会开始说另一件事——这类缺陷极难被发现，
 * 因为两处看起来都对。
 */
function classRequirementText(policy: PasswordPolicy): string {
  if (policy.min_char_classes === 1) return '字符类别不限'
  const n = CN_NUMERALS[policy.min_char_classes] ?? String(policy.min_char_classes)
  return `含大写字母、小写字母、数字、符号中的${n}类`
}

/** 命中的字符类别，与后端 `validate_password_with` 的五元组同序 */
function hitCharClasses(password: string): boolean[] {
  return [
    /[A-Z]/.test(password),
    /[a-z]/.test(password),
    /[0-9]/.test(password),
    /[^A-Za-z0-9\s]/.test(password),
    // 非 ASCII 字母（中日韩等）自成一类，与后端 is_other_script 对应
    // 用 \p{ASCII} 而不是 [^\x00-\x7F]：后者在正则里是字面控制字符，
    // eslint 的 no-control-regex 会拦下。两者语义完全一致。
    /[^\p{ASCII}]/u.test(password) && /\p{L}/u.test(password),
  ]
}

/**
 * 返回不满足规则的原因列表；空数组表示通过
 *
 * 与后端同构：长度按字符数计；字符类别分为 ASCII 大写 / ASCII 小写 /
 * 数字 / 符号 / 非 ASCII 字母五类，要求至少 `min_char_classes` 类；
 * `require_mixed_case` 是**独立的一条**，不折进类别计数——
 * 折进去会让「12 位含大小写但类别数不够」这类口令的失败原因说不清是哪一条。
 *
 * `policy` 省略时用 [`DEFAULT_PASSWORD_POLICY`]（仅供契约测试与
 * 取不到策略的降级路径；正常路径一律显式传入服务端策略）。
 */
export function passwordIssues(
  password: string,
  policy: PasswordPolicy = DEFAULT_PASSWORD_POLICY,
): string[] {
  const problems: string[] = []

  // 按字符数而非字节数：与后端保持一致，中文口令不该因编码不同得到不同结论
  const len = [...password].length
  if (len < policy.min_length) {
    problems.push(`至少 ${policy.min_length} 位`)
  }
  if (len > policy.max_length) {
    problems.push(`最多 ${policy.max_length} 位`)
  }

  const classes = hitCharClasses(password).filter(Boolean).length
  if (classes < policy.min_char_classes) {
    problems.push(`需${classRequirementText(policy)}`)
  }

  if (policy.require_mixed_case && !(/[A-Z]/.test(password) && /[a-z]/.test(password))) {
    problems.push('必须同时包含大写字母和小写字母')
  }

  return problems
}

/**
 * 把策略渲染成一句给用户看的要求
 *
 * 页面上的提示必须**从策略生成**而不是各写各的：
 * 管理员把下限从 8 调到 12，而界面上仍印着「至少 8 位」，
 * 那这条提示就变成了一个会主动误导人的装饰品。
 */
export function describePasswordPolicy(policy: PasswordPolicy): string {
  const parts = [`至少 ${policy.min_length} 位`]
  parts.push(classRequirementText(policy))
  if (policy.require_mixed_case) {
    parts.push('必须同时含大写与小写字母')
  }
  return parts.join('，')
}

/**
 * 策略是否比回落策略**更严格**
 *
 * 只认「门槛被抬高」的方向。放宽（min_length 变小）不算——
 * 它的后果是用户能设更弱的口令，而页面上没有任何东西需要为此
 * 额外解释；把它算成「更严格」会让这个判断反过来误导人。
 */
export function isStricterThanDefault(policy: PasswordPolicy): boolean {
  return (
    policy.min_length > DEFAULT_PASSWORD_POLICY.min_length ||
    policy.min_char_classes > DEFAULT_PASSWORD_POLICY.min_char_classes ||
    policy.require_mixed_case
  )
}

/**
 * 跨语言契约样例
 *
 * 格式被后端集成测试逐行解析（`{ pw: '...', ok: true|false }`），
 * **改动格式会让那条测试解析失败**——这正是它该有的行为。
 *
 * 这些样例走的是后端**默认策略**（后端那条测试调 `validate_password`），
 * 所以它们描述的是 [`DEFAULT_PASSWORD_POLICY`] 而不是任意策略。
 * 非默认策略由 `utils/__tests__/password.spec.ts` 在前端侧覆盖。
 */
export const PASSWORD_POLICY_CASES: ReadonlyArray<{ pw: string; ok: boolean }> = [
  { pw: 'admin123', ok: true },
  { pw: 'user1234', ok: true },
  { pw: 'Admin123', ok: true },
  { pw: 'admin-123', ok: true },
  { pw: 'abc123', ok: false },
  { pw: '12345678', ok: false },
  { pw: 'aaaaaaaa', ok: false },
  { pw: '!!!!!!!!', ok: false },
  { pw: '密码密码密码密码', ok: false },
  { pw: '密码密码密码密码1', ok: true },
  // emoji 属"符号"类；8 个 emoji + 1 个数字即两类
  { pw: '🔒🔒🔒🔒🔒🔒🔒🔒1', ok: true },
  // 4 个 emoji + 1 个 ASCII 字母：UTF-16 码元 9、码点 5，必须按码点判为超短
  { pw: '🔒🔒🔒🔒A', ok: false },
]
