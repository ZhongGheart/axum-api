/**
 * 权限指令 v-permission
 *
 * 根据当前用户的角色列表控制 DOM 元素显隐。
 * 无权限的元素使用 v-if 彻底移除，非 disable。
 *
 * 使用方式：
 *   <div v-permission="'admin'">仅管理员可见</div>
 *   <div v-permission="['admin', 'user']">管理员和用户可见</div>
 */
import type { Directive, DirectiveBinding } from 'vue'
import { useUserStore } from '@/stores/user'

/** 检查当前用户是否拥有指定角色 */
function hasPermission(roles: string | string[]): boolean {
  const userStore = useUserStore()
  if (!userStore.userInfo) return false

  const userRoles = userStore.userInfo.roles || [userStore.userInfo.role]
  const required = Array.isArray(roles) ? roles : [roles]

  return required.some((r) => userRoles.includes(r))
}

export const vPermission: Directive = {
  mounted(el: HTMLElement, binding: DirectiveBinding<string | string[]>) {
    if (!hasPermission(binding.value)) {
      el.parentNode?.removeChild(el)
    }
  },
  updated(el: HTMLElement, binding: DirectiveBinding<string | string[]>) {
    if (!hasPermission(binding.value)) {
      el.parentNode?.removeChild(el)
    }
  },
}
