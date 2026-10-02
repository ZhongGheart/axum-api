/**
 * 口令策略前端副本的单元测试
 *
 * 真正的价值不在"前端规则对不对"，而在它与后端的一致性——
 * 那一半由后端读本仓 `utils/password.ts` 的集成测试保证
 * （`password_policy_agrees_with_the_frontend_copy`）。
 */
import { describe, it, expect } from 'vitest'
import { passwordIssues, PASSWORD_MIN_LEN, PASSWORD_MAX_LEN } from '@/utils/password'

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
