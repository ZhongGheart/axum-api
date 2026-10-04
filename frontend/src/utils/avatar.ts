/**
 * 头像展示地址
 *
 * 后端存的是站内相对路径（`/uploads/avatars/xxx.png`）。直接塞进 `src`
 * 会指向前端 dev server 的端口而不是后端，所以必须补上 API 前缀。
 *
 * 这个拼接此前只写在 `views/profile` 里，顶栏又自己渲染了首字母头像——
 * 于是用户在个人中心换了头像，顶栏还是那个首字母。
 * 两处都走这里，"上传的头像到底在哪儿显示"就只有一个答案。
 */
export function resolveAvatarUrl(avatarUrl: string | null | undefined): string {
  if (!avatarUrl) return ''
  // 已经是绝对地址（对象存储、CDN 或外部链接）就不再加前缀
  if (/^(https?:)?\/\//.test(avatarUrl) || avatarUrl.startsWith('data:')) return avatarUrl
  const base = import.meta.env.VITE_API_BASE_URL || '/api'
  // 已经是带前缀的站内绝对路径，避免出现 `/api/api/uploads/...`
  if (avatarUrl.startsWith(base)) return avatarUrl
  return `${base}${avatarUrl.startsWith('/') ? '' : '/'}${avatarUrl}`
}
