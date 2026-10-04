/**
 * 头像地址拼接
 *
 * 盯的是**前缀重复**与**绝对地址误伤**两类事故：
 * - 站内相对路径必须补 API 前缀（否则打到前端 dev server，头像永远裂开）
 * - 已经是带前缀的路径不能再补一次（否则变成 `/api/api/uploads/...`）
 * - 绝对地址（CDN / 对象存储 / data URI）必须原样透传
 */
import { describe, expect, it } from 'vitest'
import { resolveAvatarUrl } from '@/utils/avatar'

describe('resolveAvatarUrl', () => {
  it('站内相对路径补上 API 前缀', () => {
    expect(resolveAvatarUrl('/uploads/avatars/a.png')).toBe('/api/uploads/avatars/a.png')
  })

  it('缺少前导斜杠的站内路径也能正确拼接', () => {
    expect(resolveAvatarUrl('uploads/avatars/a.png')).toBe('/api/uploads/avatars/a.png')
  })

  it('已经带 API 前缀的路径不被二次加前缀', () => {
    expect(resolveAvatarUrl('/api/uploads/avatars/a.png')).toBe('/api/uploads/avatars/a.png')
  })

  it('绝对地址原样透传，不加前缀', () => {
    expect(resolveAvatarUrl('https://cdn.example.com/a.png')).toBe('https://cdn.example.com/a.png')
    expect(resolveAvatarUrl('http://cdn.example.com/a.png')).toBe('http://cdn.example.com/a.png')
    expect(resolveAvatarUrl('//cdn.example.com/a.png')).toBe('//cdn.example.com/a.png')
  })

  it('data URI 原样透传', () => {
    expect(resolveAvatarUrl('data:image/png;base64,AAA')).toBe('data:image/png;base64,AAA')
  })

  it('没有头像时返回空串，调用方据此回退到首字母', () => {
    expect(resolveAvatarUrl(null)).toBe('')
    expect(resolveAvatarUrl(undefined)).toBe('')
    expect(resolveAvatarUrl('')).toBe('')
  })
})
