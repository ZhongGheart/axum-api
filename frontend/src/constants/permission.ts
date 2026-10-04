/**
 * 权限码常量
 *
 * 与后端 `src/model/permission.rs` 的权限码定义一一对应。
 * 由 `src/__tests__/permissionCodes.spec.ts` 做契约校验：
 * 前端引用的每个权限码都必须在后端定义表里存在，
 * 否则后端会种不出这个码，表现为「前端要授权、后端没人认」。
 */

export const PERM = {
  // 用户管理
  USER_LIST: 'system:user:list',
  USER_CREATE: 'system:user:create',
  USER_UPDATE: 'system:user:update',
  USER_DELETE: 'system:user:delete',
  /** 解锁被临时锁定的账号（v0.20.0）——与 USER_UPDATE 分开，见后端权限码注释 */
  USER_UNLOCK: 'system:user:unlock',

  /** 在线会话列举与单会话吊销（v0.20.0） */
  SESSION_MANAGE: 'system:session:manage',

  // 部门管理（v0.24.0）
  DEPT_LIST: 'system:dept:list',
  DEPT_CREATE: 'system:dept:create',
  DEPT_UPDATE: 'system:dept:update',
  DEPT_DELETE: 'system:dept:delete',

  // 角色管理
  ROLE_LIST: 'system:role:list',
  ROLE_CREATE: 'system:role:create',
  ROLE_UPDATE: 'system:role:update',
  ROLE_DELETE: 'system:role:delete',

  // 菜单管理
  MENU_LIST: 'system:menu:list',
  MENU_CREATE: 'system:menu:create',
  MENU_UPDATE: 'system:menu:update',
  MENU_DELETE: 'system:menu:delete',
  MENU_GRANT: 'system:menu:grant',

  // 字典管理
  DICT_LIST: 'system:dict:list',
  DICT_CREATE: 'system:dict:create',
  DICT_UPDATE: 'system:dict:update',
  DICT_DELETE: 'system:dict:delete',
  DICT_REFRESH: 'system:dict:refresh',

  // 系统参数（v0.22.0）
  SETTING_LIST: 'system:setting:list',
  SETTING_UPDATE: 'system:setting:update',

  // 审计日志
  LOG_LIST: 'system:log:list',
  LOG_EXPORT: 'system:log:export',

  // 系统监控
  MONITOR_SYSTEM: 'system:monitor:system',
  MONITOR_API: 'system:monitor:api',
  MONITOR_ALERT: 'system:monitor:alert',
  MONITOR_RESET: 'system:monitor:reset',
  MONITOR_EXPORT: 'system:monitor:export',

  // 其他
  EXPORT_USER: 'system:export:user',
  VALIDATE_TEST: 'system:validate:test',
  TEST_ACCESS: 'system:test:access',
} as const

/** 全部权限码 */
export const ALL_PERMISSION_CODES: string[] = Object.values(PERM)
