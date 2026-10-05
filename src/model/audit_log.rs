//! 操作日志模型

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 一条审计涉及的结构化对象引用（v0.26.0）
///
/// 与 [`AuditLog`] 的 `result` 文本**并行**而非替代：文本给人读，
/// 这里给机器筛。"谁改过 role:3 的权限"只能靠它答出来。
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct AuditLogTarget {
    pub target_type: String,
    /// 对象 ID；字符串主键的资源（系统参数）为 `None`
    pub target_id: Option<Uuid>,
    /// 字符串主键的资源的键；与 `target_id` 至少有一个非空
    pub target_key: Option<String>,
    pub change_type: String,
    /// 当时的名字；目标行已删除时靠它答出"改的是什么"
    pub target_label: Option<String>,
}

/// 从库里读 target 行时的中间形态
///
/// 与对外的 [`AuditLogTarget`] 分开：`audit_log_id` 是**关联键**，
/// 在响应里它就是噪声——每行都重复着"自己属于哪条审计"，
/// 而前端拿到整页数据时那个信息毫无用处。
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AuditLogTargetRow {
    pub audit_log_id: Uuid,
    #[sqlx(flatten)]
    pub target: AuditLogTarget,
}

/// 列表端点的一行：审计本体 + 它涉及的对象（v0.26.0）
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct AuditLogWithTargets {
    #[serde(flatten)]
    pub log: AuditLog,
    /// 该行涉及的结构化对象；历史行（结构化上线前）为空数组
    pub targets: Vec<AuditLogTarget>,
}

impl From<AuditLog> for AuditLogWithTargets {
    fn from(log: AuditLog) -> Self {
        Self {
            log,
            targets: Vec::new(),
        }
    }
}

/// 写入侧的一条结构化对象引用
///
/// 与读出侧的 [`AuditLogTarget`] 分开：写入侧用枚举（[`TargetType`] /
/// [`ChangeType`]）表达，避免 handler 传进任意字符串——
/// 那些字符串会进索引列，而 `"User"` 与 `"user"` 指同一类对象却查不到对方。
#[derive(Debug, Clone)]
pub struct AuditTargetEntry {
    pub target_type: TargetType,
    /// 对象 ID；字符串主键的资源（系统参数）为 `None`
    pub target_id: Option<Uuid>,
    /// 字符串主键的资源的键；与 `target_id` 至少有一个非空
    pub target_key: Option<String>,
    pub change_type: ChangeType,
    /// 当时的名字；目标行已删除时靠它答出"改的是什么"
    pub target_label: Option<String>,
}

impl AuditTargetEntry {
    /// 以 UUID 标识对象（绝大多数资源）
    pub fn by_id(
        target_type: TargetType,
        target_id: Uuid,
        change_type: ChangeType,
        target_label: Option<String>,
    ) -> Self {
        Self {
            target_type,
            target_id: Some(target_id),
            target_key: None,
            change_type,
            target_label,
        }
    }

    /// 以字符串键标识对象
    ///
    /// 系统参数的主键就是参数名（`security.password.min_length`），没有 UUID。
    /// 给它编一个假 UUID 会让"按参数名查审计"这条路直接断掉——
    /// 而排查"口令策略被谁改小了"时，这是唯一的入口。
    pub fn by_key(
        target_type: TargetType,
        key: impl Into<String>,
        change_type: ChangeType,
        target_label: Option<String>,
    ) -> Self {
        Self {
            target_type,
            target_id: None,
            target_key: Some(key.into()),
            change_type,
            target_label,
        }
    }
}

/// 被操作对象的种类
///
/// **收敛成枚举而不是任意字符串**：这些值会进 `audit_log_targets.target_type`
/// 并被筛选与索引。放任 handler 传任意字符串，就等于允许出现
/// `"User"` / `"user"` / `"USER"` 三种写法指同一类对象——
/// 筛选时按 `user` 查，另一两种静默查不到。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetType {
    User,
    Role,
    Menu,
    DictType,
    DictItem,
    Department,
    Setting,
    UserTwoFactor,
}

impl TargetType {
    /// 入库用的稳定字符串
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Role => "role",
            Self::Menu => "menu",
            Self::DictType => "dict_type",
            Self::DictItem => "dict_item",
            Self::Department => "department",
            Self::Setting => "setting",
            Self::UserTwoFactor => "user_two_factor",
        }
    }

    /// 解析筛选参数里的字符串
    ///
    /// **大小写与下划线宽容**：筛选框是人手输的，
    /// `User` / `USER` / `user` 应当都指同一类。解析不了返回 `None`，
    /// 让调用方回 400——静默忽略一个看不懂的筛选值，
    /// 表现就是"筛了没反应"，用户会以为没有这类记录。
    pub fn parse(s: &str) -> Option<Self> {
        let norm = s.trim().to_ascii_lowercase().replace('-', "_");
        match norm.as_str() {
            "user" | "users" => Some(Self::User),
            "role" | "roles" => Some(Self::Role),
            "menu" | "menus" => Some(Self::Menu),
            "dict_type" | "dicttype" | "dict_types" => Some(Self::DictType),
            "dict_item" | "dictitem" | "dict_items" => Some(Self::DictItem),
            "department" | "departments" | "dept" => Some(Self::Department),
            "setting" | "settings" => Some(Self::Setting),
            "user_two_factor" | "usertwofactor" | "two_factor" | "2fa" => Some(Self::UserTwoFactor),
            _ => None,
        }
    }
}

impl std::fmt::Display for TargetType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 变更类型
///
/// 同样收敛成枚举，理由见 [`TargetType`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeType {
    Create,
    Update,
    Delete,
    Grant,
    Revoke,
    Enable,
    Disable,
    /// 启用 / 停用、锁定 / 解锁这类"状态迁移"
    Status,
    /// 吊销会话
    RevokeSession,
    /// 登录（含成功、失败、待二次验证）
    ///
    /// 单独一种而不是复用 `grant`：登录并不修改任何角色或权限，
    /// 说它"授予了"会让"这个账号被谁授权过"混进一串登录记录。
    Login,
}

impl ChangeType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Update => "update",
            Self::Delete => "delete",
            Self::Grant => "grant",
            Self::Revoke => "revoke",
            Self::Enable => "enable",
            Self::Disable => "disable",
            Self::Status => "status",
            Self::RevokeSession => "revoke_session",
            Self::Login => "login",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        let norm = s.trim().to_ascii_lowercase().replace('-', "_");
        match norm.as_str() {
            "create" => Some(Self::Create),
            "update" => Some(Self::Update),
            "delete" => Some(Self::Delete),
            "grant" => Some(Self::Grant),
            "revoke" => Some(Self::Revoke),
            "enable" => Some(Self::Enable),
            "disable" => Some(Self::Disable),
            "status" => Some(Self::Status),
            "revoke_session" => Some(Self::RevokeSession),
            "login" => Some(Self::Login),
            _ => None,
        }
    }
}

impl std::fmt::Display for ChangeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 操作日志数据库实体
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct AuditLog {
    pub id: Uuid,
    pub user_id: Option<Uuid>,
    pub username: Option<String>,
    pub action: String,
    pub method: String,
    pub path: String,
    pub params: Option<String>,
    pub result: Option<String>,
    pub status_code: Option<i32>,
    pub client_ip: Option<String>,
    pub duration_ms: Option<i32>,
    pub created_at: DateTime<Utc>,
}
