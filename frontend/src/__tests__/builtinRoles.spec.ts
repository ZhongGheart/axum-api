/**
 * 内置角色名单契约测试
 *
 * 前端据此决定"是否渲染删除按钮"，后端据此拒绝删除。两份清单一旦漂移，
 * 前端就会给出一个必然 400 的按钮（或反过来该藏没藏）。
 *
 * 与 `permissionCodes.spec.ts` 同一思路：对着后端源码校验，而不是靠人记得同步。
 */
/// <reference types="node" />
import { existsSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import { describe, expect, it } from 'vitest'
import { BUILTIN_ROLE_NAMES, ADMIN_ROLE_NAME, isBuiltinRole } from '@/constants/builtin'

function readBackendRoleModel(): string {
  // jsdom 环境下 import.meta.url 是 http URL，无法 fileURLToPath；
  // 从 cwd 逐级向上找，兼容从仓库根或 frontend 目录发起测试
  let dir = process.cwd()
  for (;;) {
    const candidate = join(dir, 'src', 'model', 'role.rs')
    if (existsSync(candidate)) return readFileSync(candidate, 'utf8')
    const parent = join(dir, '..')
    if (parent === dir) {
      throw new Error(`未找到后端角色模型（自 ${process.cwd()} 向上查找）`)
    }
    dir = parent
  }
}

/** `pub const NAME: &str = "value";` → NAME → value */
function parseStrConsts(source: string): Map<string, string> {
  const map = new Map<string, string>()
  for (const m of source.matchAll(/pub const (\w+): &str = "([^"]+)"/g)) {
    map.set(m[1], m[2])
  }
  return map
}

/** `pub const BUILTIN_ROLES: [&str; N] = [A, "b"];` → ['a', 'b'] */
function parseBuiltinRoles(source: string, strConsts: Map<string, string>): string[] {
  const match = source.match(/pub const BUILTIN_ROLES: \[&str; \d+\] = \[([^\]]*)\]/)
  if (!match) throw new Error('未能在后端 model/role.rs 中解析出 BUILTIN_ROLES')
  return match[1]
    .split(',')
    .map((raw) => raw.trim())
    .filter(Boolean)
    .map((item) => {
      if (item.startsWith('"')) return item.replace(/"/g, '')
      const resolved = strConsts.get(item)
      if (resolved === undefined) {
        throw new Error(`BUILTIN_ROLES 里的 ${item} 既不是字面量也不是已知的 &str 常量`)
      }
      return resolved
    })
}

const source = readBackendRoleModel()
const strConsts = parseStrConsts(source)
const backendBuiltinRoles = parseBuiltinRoles(source, strConsts)

describe('内置角色名单与后端一致', () => {
  it('后端定义表可被解析（非空）', () => {
    expect(backendBuiltinRoles.length).toBeGreaterThan(0)
  })

  it('前端名单与后端 BUILTIN_ROLES 完全一致', () => {
    expect([...BUILTIN_ROLE_NAMES].sort()).toEqual([...backendBuiltinRoles].sort())
  })

  it('ADMIN_ROLE_NAME 与后端 ADMIN_ROLE 一致', () => {
    expect(ADMIN_ROLE_NAME).toBe(strConsts.get('ADMIN_ROLE'))
  })

  it('admin 必须在内置名单内（系统永远要留得出管理员角色）', () => {
    expect(BUILTIN_ROLE_NAMES).toContain(ADMIN_ROLE_NAME)
  })

  it('后端新增内置角色时，前端清单未同步会让本测试失败', () => {
    // 反向断言：后端清单里任何一项前端都必须认识
    const unknown = backendBuiltinRoles.filter((role) => !BUILTIN_ROLE_NAMES.includes(role))
    expect(unknown, `后端新增内置角色，前端未同步: ${unknown.join(', ')}`).toEqual([])
  })
})

describe('isBuiltinRole', () => {
  it('内置角色为 true', () => {
    for (const role of BUILTIN_ROLE_NAMES) {
      expect(isBuiltinRole(role)).toBe(true)
    }
  })

  it('业务角色为 false（因此会渲染删除按钮）', () => {
    expect(isBuiltinRole('auditor')).toBe(false)
    expect(isBuiltinRole('Admin')).toBe(false)
    expect(isBuiltinRole('')).toBe(false)
  })
})
