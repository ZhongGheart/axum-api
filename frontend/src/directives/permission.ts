/**
 * 权限指令 v-permission
 *
 * 根据当前用户的**权限码**控制 DOM 元素显隐。
 * 无权限的元素彻底从 DOM 移除（不是 disabled）。
 *
 * 使用方式：
 *   <div v-permission="'system:user:list'">仅持有查询权限时可见</div>
 *   <div v-permission="['system:user:create', 'system:user:update']">任一权限码即可见</div>
 *
 * 权限码与后端 `PermissionGuard` 同源（menus.permission 的 type='button' 行），
 * 因此「前端隐藏」与「后端 403」判定一致。
 */
import type { Directive, DirectiveBinding } from 'vue'
import { usePermissionsStore } from '@/stores/permissions'

export const vPermission: Directive = {
  mounted(el: HTMLElement, binding: DirectiveBinding<string | string[]>) {
    if (!usePermissionsStore().hasAny(binding.value)) {
      el.parentNode?.removeChild(el)
    }
  },
  updated(el: HTMLElement, binding: DirectiveBinding<string | string[]>) {
    if (!usePermissionsStore().hasAny(binding.value)) {
      el.parentNode?.removeChild(el)
    }
  },
}
