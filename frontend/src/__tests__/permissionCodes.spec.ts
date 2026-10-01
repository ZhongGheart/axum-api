/**
 * 权限码契约测试
 *
 * 前端常量与后端 `src/model/permission.rs` 必须一致：
 * 前端引用了后端没有定义的权限码，后端就种不出这一行，
 * 表现为「前端要授权、后端没人认」，按钮该藏没藏、该显不显。
 */
/// <reference types="node" />
import { existsSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import { describe, expect, it } from 'vitest'
import { ALL_PERMISSION_CODES, PERM } from '@/constants/permission'

/**
 * 仅后端使用的权限码（前端没有对应按钮）
 *
 * 显式列出而非放宽断言：后端新增权限码时，
 * 「覆盖后端定义的全部权限码」那条会失败，
 * 迫使我们明确判断「前端是否需要对应按钮」，而不是默默漂移。
 *
 * v0.7.0 起为空：原先豁免的两个码都补上了前端入口
 * （`system:monitor:export` → 系统监控页导出按钮，
 * `system:test:access` → 后端能力示例页的能力探测）。
 */
const BACKEND_ONLY_CODES: Record<string, string> = {}

/** 解析后端权限码定义表里的所有权限码 */
function backendPermissionCodes(): string[] {
  // jsdom 环境下 import.meta.url 是 http URL，无法 fileURLToPath；
  // 因此从 cwd 逐级向上找到后端定义表，兼容从仓库根或 frontend 目录发起测试
  let dir = process.cwd()
  let rustPath = ''
  for (;;) {
    const candidate = join(dir, 'src', 'model', 'permission.rs')
    if (existsSync(candidate)) {
      rustPath = candidate
      break
    }
    const parent = join(dir, '..')
    if (parent === dir) {
      throw new Error(`未找到后端权限码定义表（自 ${process.cwd()} 向上查找）`)
    }
    dir = parent
  }
  const source = readFileSync(rustPath, 'utf8')
  // 形如 `pub const USER_LIST: &str = "system:user:list";`
  const matches = source.matchAll(/pub const \w+: &str = "([^"]+)"/g)
  return [...matches].map((m) => m[1])
}

describe('前端权限码常量', () => {
  const backend = backendPermissionCodes()

  it('后端权限码定义表可被解析（非空）', () => {
    expect(backend.length).toBeGreaterThan(0)
  })

  it('前端每个权限码都在后端定义表中', () => {
    const unknown = ALL_PERMISSION_CODES.filter((code) => !backend.includes(code))
    expect(unknown, `前端引用了后端未定义的权限码: ${unknown.join(', ')}`).toEqual([])
  })

  it('权限码符合 <模块>:<资源>:<动作> 命名约定', () => {
    for (const code of ALL_PERMISSION_CODES) {
      const segments = code.split(':')
      expect(segments, `${code} 必须是三段式`).toHaveLength(3)
      expect(segments[0], `${code} 模块前缀应为 system`).toBe('system')
    }
  })

  it('前端权限码无重复', () => {
    expect(new Set(ALL_PERMISSION_CODES).size).toBe(ALL_PERMISSION_CODES.length)
  })

  it('覆盖后端定义的全部权限码（防止新增后端权限码时前端漏同步）', () => {
    const known = new Set([...ALL_PERMISSION_CODES, ...Object.keys(BACKEND_ONLY_CODES)])
    const missing = backend.filter((code) => !known.has(code))
    expect(
      missing,
      `后端新增了权限码，但既不在前端常量、也不在 BACKEND_ONLY_CODES 豁免清单里: ${missing.join(', ')}`,
    ).toEqual([])
  })

  it('豁免清单里的权限码确实存在于后端（避免豁免项写错后失去意义）', () => {
    for (const code of Object.keys(BACKEND_ONLY_CODES)) {
      expect(backend, `豁免的权限码 ${code} 在后端已不存在，请移除豁免`).toContain(code)
    }
  })

  it('PERM 键名与权限码一一对应且可读', () => {
    expect(PERM.USER_CREATE).toBe('system:user:create')
    expect(PERM.MENU_GRANT).toBe('system:menu:grant')
  })
})
