/**
 * 口令策略（前端副本）
 *
 * **后端 `src/utils/validation.rs` 的 `validate_password` 才是唯一裁决方。**
 * 这里重复实现只为即时反馈，不是安全边界。
 *
 * 重复实现必然有漂移风险，因此两侧共用一份判定样例：
 * `PASSWORD_POLICY_CASES`。后端有一条集成测试读取本文件，
 * 用 Rust 的 `validate_password` 跑同一批口令并比对结论——
 * 规则一旦在任一侧被改动而另一侧没跟上，那条测试立刻变红。
 */

/** 口令最小长度（须与后端 `PASSWORD_MIN_LEN` 一致） */
export const PASSWORD_MIN_LEN = 8

/** 口令最大长度（须与后端 `PASSWORD_MAX_LEN` 一致） */
export const PASSWORD_MAX_LEN = 128

/**
 * 返回不满足规则的原因列表；空数组表示通过
 *
 * 与后端同构：长度按字符数计；字符类分为 ASCII 大写 / ASCII 小写 /
 * 数字 / 符号 / 非 ASCII 字母五类，要求至少两类。
 */
export function passwordIssues(password: string): string[] {
  const problems: string[] = []

  // 按字符数而非字节数：与后端保持一致，中文口令不该因编码不同得到不同结论
  if ([...password].length < PASSWORD_MIN_LEN) {
    problems.push(`至少 ${PASSWORD_MIN_LEN} 位`)
  }
  if ([...password].length > PASSWORD_MAX_LEN) {
    problems.push(`最多 ${PASSWORD_MAX_LEN} 位`)
  }

  const classes = [
    /[A-Z]/.test(password),
    /[a-z]/.test(password),
    /[0-9]/.test(password),
    /[^A-Za-z0-9\s]/.test(password),
    // 非 ASCII 字母（中日韩等）自成一类，与后端 is_other_script 对应
    // 用 \p{ASCII} 而不是 [^\x00-\x7F]：后者在正则里是字面控制字符，
    // eslint 的 no-control-regex 会拦下。两者语义完全一致。
    /[^\p{ASCII}]/u.test(password) && /\p{L}/u.test(password),
  ].filter(Boolean).length

  if (classes < 2) {
    problems.push('需含大写字母、小写字母、数字、符号中的两类')
  }

  return problems
}

/**
 * 跨语言契约样例
 *
 * 格式被后端集成测试逐行解析（`{ pw: '...', ok: true|false }`），
 * **改动格式会让那条测试解析失败**——这正是它该有的行为。
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
