/**
 * 口令策略前端副本的单元测试
 *
 * 真正的价值不在"前端规则对不对"，而在它与后端的一致性——
 * 那一半由后端读本仓 `utils/password.ts` 的集成测试保证
 * （`password_policy_agrees_with_the_frontend_copy`）。
 */
import { describe, it, expect } from 'vitest'
import {
  DEFAULT_PASSWORD_POLICY,
  describePasswordPolicy,
  isStricterThanDefault,
  passwordIssues,
  PASSWORD_MIN_LEN,
  PASSWORD_MAX_LEN,
} from '@/utils/password'
import type { PasswordPolicy } from '@/utils/password'

/** 以回落策略为底，只覆盖要改的字段 */
const policy = (over: Partial<PasswordPolicy>): PasswordPolicy => ({
  ...DEFAULT_PASSWORD_POLICY,
  ...over,
})

describe('passwordIssues', () => {
  it('接受同时含小写与数字的口令', () => {
    expect(passwordIssues('admin123')).toEqual([])
    expect(passwordIssues('user1234')).toEqual([])
  })

  it('只命中一个字符类时拒绝', () => {
    // 长度够但只有一类：键盘序列与字典攻击下几乎等于没有口令
    expect(passwordIssues('12345678').join()).toContain('两类')
    expect(passwordIssues('aaaaaaaa').join()).toContain('两类')
    expect(passwordIssues('!!!!!!!!').join()).toContain('两类')
  })

  it('长度不足时报出具体下限', () => {
    expect(passwordIssues('abc123').join()).toContain(`${PASSWORD_MIN_LEN} 位`)
  })

  it('长度超上限时报出具体上限', () => {
    expect(passwordIssues('a1'.repeat(PASSWORD_MAX_LEN + 1)).join()).toContain('最多')
  })

  /**
   * 长度按**字符数**计，不是字节数。
   * 若前端按字节、后端按字符，用户会看到一个"我明明够长却不通过"的界面，
   * 而真实拒绝原因在后端，与界面对不上。
   */
  it('按字符数而非字节数计算长度', () => {
    const chinese = '密码密码密码密码' // 8 个码点
    // UTF-8 字节数是码点数的 3 倍——若按字节判，这里会被当成 24 位
    expect(new TextEncoder().encode(chinese).length).toBe(24)
    expect([...chinese].length).toBe(PASSWORD_MIN_LEN)
    // 仍因字符类不足被拒，但**不是**因为长度
    expect(passwordIssues(chinese).join()).not.toContain('位')
    // 加一个数字即两类，长度也够
    expect(passwordIssues('密码密码密码密码1')).toEqual([])
  })

  /**
   * 星平面字符（emoji）在 UTF-16 里占**两个**码元。
   * 用 `password.length` 会把它算成 2 个字符，于是 4 个 emoji + 1 个字母
   * 被算成 9 位而放行，而后端按码点算只有 5 位、应当拒绝。
   */
  it('星平面字符按码点计而非 UTF-16 码元', () => {
    const mixed = `${'🔒'.repeat(4)}A`
    expect(mixed.length).toBe(9) // UTF-16 码元：够 8 位
    expect([...mixed].length).toBe(5) // 码点：不足 8 位
    // 必须按码点报长度不足——按码元算这里会误判为已够长
    expect(passwordIssues(mixed).join()).toContain('位')

    // emoji 属"符号"类，8 个 emoji + 1 个数字即两类，合法
    expect(passwordIssues(`${'🔒'.repeat(8)}1`)).toEqual([])
  })
})

// ──────────────────────────────────────────────
// 非默认策略（v0.22.0：策略已是运行时取值）
//
// 下面这一组**后端契约测试覆盖不到**：`PASSWORD_POLICY_CASES` 描述的是
// 默认策略，后端那条测试也只跑默认策略。而前端真正的风险恰恰在
// 「管理员改了参数之后，前端这套判定还认不认得新规则」——
// 认不得的表现是界面放行、后端拒绝。
// ──────────────────────────────────────────────
describe('passwordIssues 服从传入的策略', () => {
  it('抬高最小长度后，原本合规的口令被判为过短', () => {
    // 默认策略下 admin123 合规
    expect(passwordIssues('admin123')).toEqual([])
    // min_length = 12 时同一个口令不再合规
    expect(passwordIssues('admin123', policy({ min_length: 12 })).join()).toContain('至少 12 位')
  })

  it('降低最小长度后，短口令变得合规（默认策略下它是过短的）', () => {
    // 'ab12' = 小写 + 数字，两类；默认策略下因长度不足被拒
    expect(passwordIssues('ab12', DEFAULT_PASSWORD_POLICY).join()).toContain(`至少 ${PASSWORD_MIN_LEN} 位`)
    expect(passwordIssues('ab12', policy({ min_length: 4 }))).toEqual([])
  })

  it('提高字符类别数后，两类字符的口令被拒', () => {
    // admin123 = 小写 + 数字，两类
    expect(passwordIssues('admin123', policy({ min_char_classes: 2 }))).toEqual([])
    expect(passwordIssues('admin123', policy({ min_char_classes: 3 })).join()).toContain('三类')
  })

  it('类别数设为 1 时纯字母口令也合规', () => {
    expect(passwordIssues('aaaaaaaa', policy({ min_char_classes: 1 }))).toEqual([])
  })

  /**
   * 大小写混合是**独立的一条**，不折进类别计数。
   *
   * 折进去的话，`password123` 在开启开关后命中「大写?没有」——
   * 而它按类别数算仍是两类合法，于是开关看起来没生效。
   * 后端把它拆成两次判定（见 `validate_password_with` 的注释），
   * 前端必须同样拆开，否则两边对同一个口令给出不同结论。
   */
  it('require_mixed_case 开启后不含大写的口令被拒，即便类别数够', () => {
    const strict = policy({ require_mixed_case: true })
    // 两类（小写 + 数字），但没有大写
    expect(passwordIssues('password123', strict).join()).toContain('大写')
    // 补一个大写即可通过
    expect(passwordIssues('Password123', strict)).toEqual([])
  })

  it('纯中文口令在开启大小写混合后必然被拒——这是策略的后果，不是缺陷', () => {
    // 汉字不含 ASCII 大小写。该参数 description 里写明了这一点。
    expect(passwordIssues('密码密码密码1', policy({ require_mixed_case: true })).join())
      .toContain('大写')
  })

  it('max_length 收窄后，超长口令被拒', () => {
    expect(passwordIssues('a1'.repeat(20), policy({ max_length: 16 })).join()).toContain('最多 16 位')
  })

  it('回落策略与后端默认值一致（提示文本里的数字不能说谎）', () => {
    expect(DEFAULT_PASSWORD_POLICY.min_length).toBe(PASSWORD_MIN_LEN)
    expect(DEFAULT_PASSWORD_POLICY.max_length).toBe(PASSWORD_MAX_LEN)
  })
})

describe('describePasswordPolicy', () => {
  it('默认策略生成与旧文案等价的提示', () => {
    expect(describePasswordPolicy(DEFAULT_PASSWORD_POLICY)).toContain('至少 8 位')
    expect(describePasswordPolicy(DEFAULT_PASSWORD_POLICY)).toContain('两类')
  })

  it('策略被抬高后提示跟着变——提示里的数字必须是当前生效的那一份', () => {
    const text = describePasswordPolicy(policy({ min_length: 16, min_char_classes: 3 }))
    expect(text).toContain('至少 16 位')
    expect(text).toContain('三类')
    expect(text).not.toContain('至少 8 位')
  })

  it('开启大小写混合后提示里必须提到它', () => {
    expect(describePasswordPolicy(policy({ require_mixed_case: true }))).toContain('大写')
  })

  it('类别数为 1 时明说不限，而不是印一句含混的「至少一类」', () => {
    expect(describePasswordPolicy(policy({ min_char_classes: 1 }))).toContain('不限')
  })
})

describe('isStricterThanDefault', () => {
  it('默认策略不算更严格', () => {
    expect(isStricterThanDefault(DEFAULT_PASSWORD_POLICY)).toBe(false)
  })

  it('放宽策略也不算更严格', () => {
    expect(isStricterThanDefault(policy({ min_length: 6 }))).toBe(false)
  })

  it('抬高门槛或开启大小写混合都算更严格', () => {
    expect(isStricterThanDefault(policy({ min_length: 12 }))).toBe(true)
    expect(isStricterThanDefault(policy({ min_char_classes: 3 }))).toBe(true)
    expect(isStricterThanDefault(policy({ require_mixed_case: true }))).toBe(true)
  })
})
