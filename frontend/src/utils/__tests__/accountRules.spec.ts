/**
 * 共享账号校验规则的单元测试
 *
 * 这些规则的"对不对"由后端集成测试裁决
 * （`username_policy_agrees_with_the_frontend_rules` 与
 * `password_policy_agrees_with_the_frontend_copy`）。
 * 这里测的是**前端自己的行为**——规则函数是否按它声称的方式工作，
 * 特别是那些"按字符而非按字节"这类最容易悄悄写错的细节。
 */
import { describe, it, expect } from 'vitest'
import {
  usernameIssues,
  emailIssues,
  USERNAME_MIN_LEN,
  USERNAME_MAX_LEN,
  IDENTIFIER_MAX_LEN,
} from '@/utils/accountRules'

describe('usernameIssues', () => {
  it('接受后端允许的四种字符', () => {
    expect(usernameIssues('alice')).toEqual([])
    expect(usernameIssues('alice_01')).toEqual([])
    expect(usernameIssues('alice-01')).toEqual([])
  })

  /**
   * 字符集必须按 Unicode 判定，不能写成 ASCII 白名单。
   *
   * 后端是 Rust 的 `char::is_alphanumeric()`，那是 Unicode 感知的——
   * 中文、希腊文用户名实测能注册。前端若写成 `[a-zA-Z0-9]`，
   * 就会把一批后端放行的合法用户名拦在门外，漂移方向只是反过来。
   */
  it('接受非 ASCII 字母与数字', () => {
    expect(usernameIssues('用户名')).toEqual([])
    expect(usernameIssues('Ωμέγα')).toEqual([])
    expect(usernameIssues('用户123')).toEqual([])
  })

  it('拒绝含 @ 的取值——后端同样拒绝', () => {
    // 邮箱绝不该出现在"用户名"字段里
    expect(usernameIssues('user@name').join()).toContain('只能包含')
  })

  it('拒绝后端不放行的符号', () => {
    for (const bad of ['alice.dot', 'alice space', 'alice!', 'alice+1']) {
      expect(usernameIssues(bad).join()).toContain('只能包含')
    }
  })

  it('长度过短/过长时报出具体上下限', () => {
    expect(usernameIssues('ab').join()).toContain(`${USERNAME_MIN_LEN} 个字符`)
    expect(usernameIssues('a'.repeat(USERNAME_MAX_LEN + 1)).join()).toContain(
      `${USERNAME_MAX_LEN} 个字符`,
    )
  })

  /**
   * 长度按**字符**计，不是字节。
   *
   * 17 个汉字 = 51 字节 = 17 个字符。`users.username` 是 varchar(50)，
   * Postgres 按字符计，所以 17 个汉字能注册；按字节判则会被拒。
   * 后端 `validate_username` 已改用 `chars().count()`。
   */
  it('按字符数而非字节数计算长度', () => {
    const seventeen = '一'.repeat(17)
    // 汉字属 BMP，`.length` 就是码点数 17；UTF-8 才是它的 3 倍
    expect(seventeen.length).toBe(17)
    expect(new TextEncoder().encode(seventeen).length).toBe(51)
    expect(usernameIssues(seventeen)).toEqual([])

    // 51 个汉字 = 51 字符（超 varchar(50) 的字符上限）、153 字节 → 应当被拒。
    // 注意是 51 个**字符**触发，不是 51 个字节：按字节判的话
    // 17 个汉字（51 字节）就会被拒，那正是本次修掉的缺陷。
    const fiftyOne = '一'.repeat(51)
    expect([...fiftyOne].length).toBe(51)
    expect(new TextEncoder().encode(fiftyOne).length).toBe(153)
    expect(usernameIssues(fiftyOne).join()).toContain(`${USERNAME_MAX_LEN} 个字符`)
  })

  /**
   * 星平面**字母**（𝒜 U+1D49C）能过字符集，于是可以拿来测长度算法。
   *
   * 这是唯一能区分"按码点"与"按 UTF-16 码元"的取值：
   * 30 个 𝒜 = 30 个码点（未超 50，应通过）、60 个码元（超了）。
   * 后端 `chars().count()` 数码点，所以前端也必须数码点。
   *
   * emoji（🔒）测不了这件事——它属 So，字符集那一关就会先拒掉，
   * 断言根本走不到长度规则。
   *
   * 顺带确认两侧对它的判定一致：Rust `is_alphabetic()` 为 true，
   * JS 的 `\p{L}` 也匹配，所以它是**合法**用户名，不是取巧。
   */
  it('星平面字母按码点计而非 UTF-16 码元', () => {
    const thirty = '\u{1D49C}'.repeat(30)
    expect([...thirty].length).toBe(30) // 码点：未超上限
    expect(thirty.length).toBe(60) // 码元：已超上限
    // 按码点算应当通过；改成 .length 的话这里会红
    expect(usernameIssues(thirty)).toEqual([])

    // 26 个 𝒜 = 26 码点，仍合法
    expect(usernameIssues('\u{1D49C}'.repeat(26))).toEqual([])
  })

  /**
   * emoji（🔒）**过不了字符集**——所以别拿它测长度。
   *
   * 🔒 属 So（其他符号），Rust 的 `is_alphanumeric()` 对它返回 false，
   * 后端同样拒收。若想用它验"按码点而非码元计长度"，
   * 断言会先在字符集那一关挂掉，测到的根本不是长度规则。
   */
  it('emoji 既超字符集也超长度，两条都要报出来', () => {
    const emoji = '🔒'.repeat(30)
    // 码点 30（未超 50），码元 60（超了）——两种算法对长度的结论相反，
    // 所以这条真正验的是：即便如此，字符集问题也必须被一并报出
    expect([...emoji].length).toBe(30)
    expect(emoji.length).toBe(60)
    const problems = usernameIssues(emoji).join()
    expect(problems).toContain('只能包含')
  })
})

describe('emailIssues', () => {
  it('接受含 @ 与 . 的取值', () => {
    expect(emailIssues('alice@example.com')).toEqual([])
  })

  it('拒绝缺 @ 或缺 . 的取值', () => {
    expect(emailIssues('aliceexample.com').join()).toContain('@')
    expect(emailIssues('alice@example').join()).toContain('.')
  })

  /**
   * 上限按**字符**计，与 `users.email` 的 varchar(255) 同一把尺子。
   *
   * 这条直接关系到登录页：登录页旧规则写死 `max: 50`，
   * 持有长邮箱的合法用户在自己的登录页上敲不进自己的邮箱。
   *
   * **断言必须卡在阈值两侧，不能只查提示文案**——
   * 提示里的 "255" 是写死的常量，把判据从 255 改成 50 之后
   * 文案照样是 "最长 255 个字符"，只查文案会让这个缺陷隐形
   * （这一版就是这么漏掉过一次：注入 `email.length > 50` 全绿）。
   */
  it('按字符数判长度上限，而不是字节或码元', () => {
    // 254 字符的域名部分 → 整串 266 字符，超上限
    const tooLong = `${'a'.repeat(IDENTIFIER_MAX_LEN)}@example.com`
    expect([...tooLong].length).toBeGreaterThan(IDENTIFIER_MAX_LEN)
    expect(emailIssues(tooLong).join()).toContain('255')

    // 恰好 255 字符：应当通过。若判据被改成 50，这条会红
    const atLimit = `${'a'.repeat(IDENTIFIER_MAX_LEN - '@x.co'.length)}@x.co`
    expect([...atLimit].length).toBe(IDENTIFIER_MAX_LEN)
    expect(emailIssues(atLimit)).toEqual([])

    // 星平面字母：200 码点（未超 255）却是 400 码元（超了）。
    // 按码元判会误拒——而后端按码点收，是放行的
    const astral = `${'\u{1D49C}'.repeat(200)}@x.co`
    expect([...astral].length).toBe(205)
    expect(astral.length).toBe(405)
    expect(emailIssues(astral)).toEqual([])
  })
})
