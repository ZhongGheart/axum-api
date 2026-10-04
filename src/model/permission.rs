//! 权限码定义（单一数据源）
//!
//! 权限码保存在 `menus.permission` 列，对应 `type = 'button'` 的菜单行，
//! 通过既有的 `role_menus` 关联授权。不引入第二套权限表。
//!
//! 本模块是权限码的唯一来源：
//!
//! - handler 用这里的 const 声明自己需要的权限码
//! - 启动种子由 [`PERMISSION_DEFS`] 循环生成，保证"接口声明的码"与"种子写入的码"不会漂移
//! - 前端 `v-permission` / `PermissionButton` 使用同一套字符串（经 `/api/auth/permissions` 下发）
//!
//! 命名约定：`<模块>:<资源>:<动作>`，全部小写冒号分隔。

// ── 用户管理 ────────────────────────────────────────────────
/// 查看用户列表
pub const USER_LIST: &str = "system:user:list";
/// 创建用户
pub const USER_CREATE: &str = "system:user:create";
/// 修改用户（含分配角色、重置密码、启停、批量删除）
pub const USER_UPDATE: &str = "system:user:update";
/// 删除用户
pub const USER_DELETE: &str = "system:user:delete";
/// 解锁被登录爆破防护临时锁定的账号
///
/// v0.20.0 新增。**独立于 `USER_UPDATE`**，理由不是"粒度更细更好"，
/// 而是这个动作本身该被单独收回：`USER_UPDATE` 是日常高频操作，
/// 几乎一定会授给管理员；而解锁意味着"我确认这个人是本人"，
/// 是个低频但高判断的动作，不该与"改个显示名"共用一个开关。
pub const USER_UNLOCK: &str = "system:user:unlock";

// ── 会话管理 ────────────────────────────────────────────────
/// 查看在线会话与吊销指定会话
///
/// v0.20.0 新增。**独立于 `USER_UPDATE`**，理由同 `USER_UNLOCK`：
/// "看某人在哪些设备登录"会暴露登录时间与 IP，属于侦察面，
/// 不该与"改个显示名"共用一个开关——后者几乎必然要授给管理员。
pub const SESSION_MANAGE: &str = "system:session:manage";

// ── 部门管理 ────────────────────────────────────────────────
/// 查看部门树
pub const DEPT_LIST: &str = "system:dept:list";
/// 新建部门
pub const DEPT_CREATE: &str = "system:dept:create";
/// 修改部门
pub const DEPT_UPDATE: &str = "system:dept:update";
/// 删除部门
pub const DEPT_DELETE: &str = "system:dept:delete";

// ── 角色管理 ────────────────────────────────────────────────
/// 查看角色列表
pub const ROLE_LIST: &str = "system:role:list";
/// 创建角色
pub const ROLE_CREATE: &str = "system:role:create";
/// 修改角色
pub const ROLE_UPDATE: &str = "system:role:update";
/// 删除角色
pub const ROLE_DELETE: &str = "system:role:delete";

// ── 菜单管理 ────────────────────────────────────────────────
/// 查看菜单树
pub const MENU_LIST: &str = "system:menu:list";
/// 创建菜单/按钮
pub const MENU_CREATE: &str = "system:menu:create";
/// 修改菜单/按钮
pub const MENU_UPDATE: &str = "system:menu:update";
/// 删除菜单/按钮
pub const MENU_DELETE: &str = "system:menu:delete";
/// 分配角色菜单权限
pub const MENU_GRANT: &str = "system:menu:grant";

// ── 字典管理 ────────────────────────────────────────────────
/// 查看字典类型与条目
pub const DICT_LIST: &str = "system:dict:list";
/// 创建字典类型/条目
pub const DICT_CREATE: &str = "system:dict:create";
/// 修改字典类型/条目
pub const DICT_UPDATE: &str = "system:dict:update";
/// 删除字典类型/条目
pub const DICT_DELETE: &str = "system:dict:delete";
/// 刷新字典缓存
pub const DICT_REFRESH: &str = "system:dict:refresh";

// ── 系统参数 ────────────────────────────────────────────────
/// 查看系统参数
pub const SETTING_LIST: &str = "system:setting:list";
/// 修改系统参数
pub const SETTING_UPDATE: &str = "system:setting:update";

// ── 审计日志 ────────────────────────────────────────────────
/// 查看审计日志
pub const LOG_LIST: &str = "system:log:list";
/// 导出审计日志
pub const LOG_EXPORT: &str = "system:log:export";

// ── 系统监控 ────────────────────────────────────────────────
/// 查看系统信息
pub const MONITOR_SYSTEM: &str = "system:monitor:system";
/// 查看接口性能指标
pub const MONITOR_API: &str = "system:monitor:api";
/// 查看告警
pub const MONITOR_ALERT: &str = "system:monitor:alert";
/// 重置接口性能指标
pub const MONITOR_RESET: &str = "system:monitor:reset";
/// 导出系统信息
pub const MONITOR_EXPORT: &str = "system:monitor:export";

// ── 其他管理能力 ────────────────────────────────────────────
/// 导出用户数据
pub const EXPORT_USER: &str = "system:export:user";
/// 访问权限探测端点
pub const TEST_ACCESS: &str = "system:test:access";
/// 调用参数校验演示端点
pub const VALIDATE_TEST: &str = "system:validate:test";

/// 权限码元数据
///
/// `parent_path` 指向该按钮所属的页面菜单（`menus.path`），启动时据此解析
/// `parent_id`，使"菜单管理"页面里按钮与页面天然成树。
#[derive(Debug, Clone, Copy)]
pub struct PermissionDef {
    /// 权限码，写入 `menus.permission`
    pub code: &'static str,
    /// 按钮显示名，写入 `menus.name`
    pub name: &'static str,
    /// 所属页面菜单的 `path`
    pub parent_path: &'static str,
}

/// 全部权限码定义
///
/// 顺序即"菜单管理"页面里的展示顺序（按 `sort_order` 写入）。
pub const PERMISSION_DEFS: &[PermissionDef] = &[
    // 用户管理
    PermissionDef {
        code: USER_LIST,
        name: "查询用户",
        parent_path: "/system/user",
    },
    PermissionDef {
        code: USER_CREATE,
        name: "新建用户",
        parent_path: "/system/user",
    },
    PermissionDef {
        code: USER_UPDATE,
        name: "编辑用户",
        parent_path: "/system/user",
    },
    PermissionDef {
        code: USER_DELETE,
        name: "删除用户",
        parent_path: "/system/user",
    },
    PermissionDef {
        code: USER_UNLOCK,
        name: "解锁账号",
        parent_path: "/system/user",
    },
    PermissionDef {
        code: SESSION_MANAGE,
        name: "查看与吊销会话",
        parent_path: "/system/user",
    },
    // 部门管理
    PermissionDef {
        code: DEPT_LIST,
        name: "查询部门",
        parent_path: "/system/dept",
    },
    PermissionDef {
        code: DEPT_CREATE,
        name: "新建部门",
        parent_path: "/system/dept",
    },
    PermissionDef {
        code: DEPT_UPDATE,
        name: "编辑部门",
        parent_path: "/system/dept",
    },
    PermissionDef {
        code: DEPT_DELETE,
        name: "删除部门",
        parent_path: "/system/dept",
    },
    // 角色管理
    PermissionDef {
        code: ROLE_LIST,
        name: "查询角色",
        parent_path: "/system/role",
    },
    PermissionDef {
        code: ROLE_CREATE,
        name: "新建角色",
        parent_path: "/system/role",
    },
    PermissionDef {
        code: ROLE_UPDATE,
        name: "编辑角色",
        parent_path: "/system/role",
    },
    PermissionDef {
        code: ROLE_DELETE,
        name: "删除角色",
        parent_path: "/system/role",
    },
    // 菜单管理
    PermissionDef {
        code: MENU_LIST,
        name: "查询菜单",
        parent_path: "/system/menu",
    },
    PermissionDef {
        code: MENU_CREATE,
        name: "新建菜单",
        parent_path: "/system/menu",
    },
    PermissionDef {
        code: MENU_UPDATE,
        name: "编辑菜单",
        parent_path: "/system/menu",
    },
    PermissionDef {
        code: MENU_DELETE,
        name: "删除菜单",
        parent_path: "/system/menu",
    },
    PermissionDef {
        code: MENU_GRANT,
        name: "分配菜单权限",
        parent_path: "/system/menu",
    },
    // 字典管理
    PermissionDef {
        code: DICT_LIST,
        name: "查询字典",
        parent_path: "/system/dict",
    },
    PermissionDef {
        code: DICT_CREATE,
        name: "新建字典",
        parent_path: "/system/dict",
    },
    PermissionDef {
        code: DICT_UPDATE,
        name: "编辑字典",
        parent_path: "/system/dict",
    },
    PermissionDef {
        code: DICT_DELETE,
        name: "删除字典",
        parent_path: "/system/dict",
    },
    PermissionDef {
        code: DICT_REFRESH,
        name: "刷新字典缓存",
        parent_path: "/system/dict",
    },
    // 系统日志
    PermissionDef {
        code: SETTING_LIST,
        name: "查询系统参数",
        parent_path: "/system/setting",
    },
    PermissionDef {
        code: SETTING_UPDATE,
        name: "修改系统参数",
        parent_path: "/system/setting",
    },
    PermissionDef {
        code: LOG_LIST,
        name: "查询日志",
        parent_path: "/system/log",
    },
    PermissionDef {
        code: LOG_EXPORT,
        name: "导出日志",
        parent_path: "/system/log",
    },
    // 系统监控
    PermissionDef {
        code: MONITOR_SYSTEM,
        name: "查看系统信息",
        parent_path: "/system/monitor/system",
    },
    PermissionDef {
        code: MONITOR_API,
        name: "查看接口指标",
        parent_path: "/system/monitor/api",
    },
    PermissionDef {
        code: MONITOR_ALERT,
        name: "查看告警",
        parent_path: "/system/monitor/system",
    },
    PermissionDef {
        code: MONITOR_RESET,
        name: "重置接口指标",
        parent_path: "/system/monitor/api",
    },
    PermissionDef {
        code: MONITOR_EXPORT,
        name: "导出系统信息",
        parent_path: "/system/monitor/system",
    },
    // 其他
    PermissionDef {
        code: EXPORT_USER,
        name: "导出用户数据",
        parent_path: "/system/user",
    },
    PermissionDef {
        code: TEST_ACCESS,
        name: "访问能力探测",
        parent_path: "/",
    },
    PermissionDef {
        code: VALIDATE_TEST,
        name: "参数校验演示",
        parent_path: "/",
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn permission_codes_are_unique() {
        let mut seen = HashSet::new();
        for def in PERMISSION_DEFS {
            assert!(
                seen.insert(def.code),
                "权限码重复: {}（名称 {}）",
                def.code,
                def.name
            );
        }
        assert_eq!(seen.len(), PERMISSION_DEFS.len());
    }

    #[test]
    fn permission_codes_follow_naming_convention() {
        for def in PERMISSION_DEFS {
            let segments: Vec<&str> = def.code.split(':').collect();
            assert_eq!(
                segments.len(),
                3,
                "权限码必须是 <模块>:<资源>:<动作> 三段: {}",
                def.code
            );
            assert_eq!(
                segments[0], "system",
                "权限码模块前缀应统一为 system: {}",
                def.code
            );
            for segment in &segments {
                assert!(
                    segment
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c == ':' || c == '-'),
                    "权限码只允许小写字母/连字符: {}",
                    def.code
                );
            }
            assert!(!def.name.is_empty(), "权限码 {} 缺少按钮名称", def.code);
            assert!(
                def.parent_path.starts_with('/'),
                "父菜单路径应以 / 开头: {}",
                def.parent_path
            );
        }
    }
}
