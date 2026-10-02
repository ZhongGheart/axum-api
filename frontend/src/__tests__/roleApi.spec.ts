/**
 * 角色管理 API 契约
 *
 * 重点验证 `assignMenus` 的**请求体形状**（`menu_ids`）与"全量覆盖"语义：
 * 一旦这里传成增量或字段名写错，撤销授权就会静默失效——
 * 与 v0.5.0 修掉的后端 `.ok()` 吞错是同一类后果。
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'

// vi.mock 会被提升到文件顶部，被它引用的 mock 函数必须一起提升
const { get, post, put, del } = vi.hoisted(() => ({
  get: vi.fn(),
  post: vi.fn(),
  put: vi.fn(),
  del: vi.fn(),
}))

vi.mock('@/api/index', () => ({
  default: { get, post, put, delete: del },
}))

import { roleApi } from '@/api/role'
import { menuApi } from '@/api/menu'

const ROLE_ID = '11111111-2222-3333-4444-555555555555'

beforeEach(() => {
  get.mockReset()
  post.mockReset()
  put.mockReset()
  del.mockReset()
})

describe('roleApi 路由与请求体', () => {
  it('list 打 GET /admin/roles 并带分页参数', () => {
    roleApi.list({ page: 2, page_size: 20 })
    expect(get).toHaveBeenCalledWith('/admin/roles', { params: { page: 2, page_size: 20 } })
  })

  it('create 打 POST /admin/roles 并带 name/description', () => {
    roleApi.create({ name: 'auditor', description: '审计' })
    expect(post).toHaveBeenCalledWith('/admin/roles', { name: 'auditor', description: '审计' })
  })

  it('create 省略 description 时不发送该字段（后端 Option<String> 收到 null）', () => {
    roleApi.create({ name: 'auditor' })
    expect(post).toHaveBeenCalledWith('/admin/roles', { name: 'auditor' })
    expect(Object.keys(post.mock.calls[0][1])).not.toContain('description')
  })

  it('update 打 PUT /admin/roles/:id', () => {
    roleApi.update(ROLE_ID, { name: 'auditor2' })
    expect(put).toHaveBeenCalledWith(`/admin/roles/${ROLE_ID}`, { name: 'auditor2' })
  })

  it('delete 打 DELETE /admin/roles/:id', () => {
    roleApi.delete(ROLE_ID)
    expect(del).toHaveBeenCalledWith(`/admin/roles/${ROLE_ID}`)
  })

  it('assignMenus 打 PUT /admin/roles/:id/menus，字段名为 menu_ids', () => {
    roleApi.assignMenus(ROLE_ID, ['m1', 'm2'])
    expect(put).toHaveBeenCalledWith(`/admin/roles/${ROLE_ID}/menus`, { menu_ids: ['m1', 'm2'] })
  })

  it('assignMenus 提交调用方给定的完整集合，不自行做增删', () => {
    roleApi.assignMenus(ROLE_ID, ['m1'])
    // 只有 m1：后端会撤销其余既有授权，语义由调用方保证提交完整集合
    expect(put).toHaveBeenCalledWith(`/admin/roles/${ROLE_ID}/menus`, { menu_ids: ['m1'] })
  })

  it('assignMenus 允许提交空集合（撤销全部授权）', () => {
    roleApi.assignMenus(ROLE_ID, [])
    expect(put).toHaveBeenCalledWith(`/admin/roles/${ROLE_ID}/menus`, { menu_ids: [] })
  })

  it('既有用户角色分配接口保持不变', () => {
    roleApi.getUserRoles(ROLE_ID)
    roleApi.assignRole(ROLE_ID, 'admin')
    expect(get).toHaveBeenCalledWith(`/admin/users/${ROLE_ID}/roles`)
    expect(post).toHaveBeenCalledWith(`/admin/users/${ROLE_ID}/roles`, {
      user_id: ROLE_ID,
      role_name: 'admin',
    })
  })
  it('listByRole 以 query 参数传 role_id，而不是拼进路径', () => {
    menuApi.listByRole(ROLE_ID)
    expect(get).toHaveBeenCalledWith('/admin/menus', { params: { role_id: ROLE_ID } })
  })
})

describe('roleApi.listAll 的翻页', () => {
  /** 造一页响应 */
  const pageOf = (names: string[], total: number) => ({
    items: names.map((name) => ({ id: name, name, description: null, created_at: '', user_count: 0 })),
    total,
    page: 1,
    page_size: 200,
    total_pages: 1,
  })

  it('单页取完时只请求一次', async () => {
    get.mockResolvedValue(pageOf(['admin', 'user'], 2))
    const roles = await roleApi.listAll()
    expect(roles.map((r) => r.name)).toEqual(['admin', 'user'])
    expect(get).toHaveBeenCalledTimes(1)
  })

  it('多页时翻到取完为止', async () => {
    // 总量 250 > 单页 200，必须再取一页，否则下拉会少 50 个角色
    get.mockResolvedValueOnce(pageOf(Array.from({ length: 200 }, (_, i) => `r${i}`), 250))
    get.mockResolvedValueOnce(pageOf(Array.from({ length: 50 }, (_, i) => `s${i}`), 250))
    const roles = await roleApi.listAll()
    expect(roles).toHaveLength(250)
    expect(get).toHaveBeenCalledTimes(2)
    expect(get.mock.calls[1][1]).toEqual({ params: { page: 2, page_size: 200 } })
  })

  it('总数与实际取到的条数对不上时抛错，而不是悄悄截断', async () => {
    // 后端说有 10 条却一直返回空页：静默返回 0 条会让下拉变成空的
    get.mockResolvedValue(pageOf([], 10))
    await expect(roleApi.listAll()).rejects.toThrow(/分页异常/)
  })
})
