/**
 * 系统参数 API
 *
 * 对应后端 `controller/setting.rs`。
 *
 * **参数清单不在前端硬编码**：能改哪些参数、类型、取值范围、默认值
 * 全部由后端 `SETTING_DEFS` 定义并在列表响应里带回来。
 * 前端只负责按后端给的形状渲染控件——写死一份清单等于把
 * 「参数定义」这个单一数据源劈成两半，后端加一个参数而前端没跟上时，
 * 表现是「后端支持、前端看不见」，管理员会以为没生效。
 */

import http from './index'

/** 参数分组（后端 `SettingGroup`） */
export type SettingGroup = 'password' | 'login'

/** 参数取值类型（后端 `SettingType::as_str`） */
export type SettingValueType = 'int' | 'bool'

/** 生效取值的来源：管理员显式改过 / 部署配置 / 代码默认值 */
export type SettingSource = 'admin' | 'env' | 'default'

/** 系统参数列表项 */
export interface SettingItem {
  /** 参数名（DB 主键） */
  key: string
  /** 界面上显示的名称 */
  name: string
  /** 用途说明：写的是「改了会发生什么」 */
  description: string
  /** 取值类型 */
  value_type: SettingValueType
  /** 分组 */
  group: SettingGroup
  /** 当前**实际生效**的取值（文本） */
  value: string
  /** 代码默认值（文本） */
  default: string
  /** 整数下界；bool 参数忽略 */
  min: number
  /** 整数上界；bool 参数忽略 */
  max: number
  /** 该参数被哪段代码消费 */
  consumed_by: string
  /** 当前落库取值是否等于默认值 */
  is_default: boolean
  /** 是否被管理员显式改过（决定参数表压不压部署配置） */
  admin_overridden: boolean
  /** 生效取值的来源 */
  source: SettingSource
  /** 最近修改者 */
  updated_by: string | null
  /** 最近修改时间 */
  updated_at: string
}

/** 修改单个参数的请求体 */
export interface UpdateSettingReq {
  value: string
}

/**
 * 面向未登录页面的口令策略
 *
 * 字段与后端 `PublicPasswordPolicy` 逐字对应。
 * **刻意不含** `expiry_days` / 锁定阈值：这些对访客无价值，
 * 而把「账号多久被锁一次」暴露给未登录端点等于给爆破者一个可调的参数面板。
 */
export interface PublicPasswordPolicy {
  min_length: number
  max_length: number
  min_char_classes: number
  require_mixed_case: boolean
}

export const settingApi = {
  /** 列出全部系统参数（需 `system:setting:list`） */
  list(): Promise<SettingItem[]> {
    return http.get('/admin/settings') as unknown as Promise<SettingItem[]>
  },

  /** 修改单个参数（需 `system:setting:update`） */
  update(key: string, value: string): Promise<SettingItem> {
    return http.put(`/admin/settings/${encodeURIComponent(key)}`, {
      value,
    }) as unknown as Promise<SettingItem>
  },

  /** 复位成默认值（需 `system:setting:update`） */
  reset(key: string): Promise<SettingItem> {
    return http.post(
      `/admin/settings/${encodeURIComponent(key)}/reset`,
    ) as unknown as Promise<SettingItem>
  },

  /** 清理参数缓存（需 `system:setting:update`） */
  refreshCache(): Promise<string> {
    return http.post('/admin/settings/refresh-cache') as unknown as Promise<string>
  },

  /**
   * 当前口令策略（**公开端点**，无需登录）
   *
   * 注册页与改密页要在**用户动手之前**告诉他规则。
   * 取不到时前端回落到内置默认策略：宁可提示得不准一点，
   * 也不能因为一个可选的提示接口挂了就拦住注册——
   * 后端始终是裁决方，它会带着准确的规则拒绝不合规的口令。
   */
  passwordPolicy(): Promise<PublicPasswordPolicy> {
    return http.get('/settings/password-policy') as unknown as Promise<PublicPasswordPolicy>
  },
}
