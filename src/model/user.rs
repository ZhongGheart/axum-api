//! 用户数据模型
//!
//! 对应数据库中 `users` 表的实体结构。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 用户主角色
///
/// 角色数据的唯一来源是 `user_roles` 表；本枚举只用于对外表达"主角色"，
/// 权限判定必须使用 `UserInfo::roles`，不要依赖该枚举。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// 管理员
    Admin,
    /// 普通用户
    User,
}

impl Role {
    /// 计算主角色：拥有 admin 即视为管理员，否则为普通用户
    pub fn primary_from(roles: &[String]) -> Role {
        if roles.iter().any(|r| r.eq_ignore_ascii_case("admin")) {
            Role::Admin
        } else {
            Role::User
        }
    }
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Role::Admin => write!(f, "admin"),
            Role::User => write!(f, "user"),
        }
    }
}

/// 用户数据库实体
///
/// 映射 `users` 表的每一行记录。
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct User {
    /// 用户唯一标识（UUID v4）
    pub id: Uuid,
    /// 用户名，唯一
    pub username: String,
    /// 电子邮箱，唯一
    pub email: String,
    /// 密码哈希值（Argon2 加密）
    pub password_hash: String,
    /// 是否激活
    pub is_active: bool,
    /// 创建时间
    pub created_at: DateTime<Utc>,
    /// 更新时间
    pub updated_at: DateTime<Utc>,
}

/// 用户注册请求体
#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    /// 用户名（3-50 个字符）
    pub username: String,
    /// 电子邮箱
    pub email: String,
    /// 密码（至少 6 个字符）
    pub password: String,
}

/// 用户登录请求体
#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    /// 用户名或邮箱
    pub username: String,
    /// 密码
    pub password: String,
}

/// 登录成功响应
#[derive(Debug, Serialize)]
pub struct LoginResponse {
    /// JWT 访问令牌
    pub token: String,
    /// 令牌类型
    pub token_type: String,
}

/// 当前用户信息（对外暴露，不含密码）
#[derive(Debug, Serialize)]
pub struct UserInfo {
    /// 用户唯一标识
    pub id: Uuid,
    /// 用户名
    pub username: String,
    /// 电子邮箱
    pub email: String,
    /// 主角色（由 `roles` 计算得到，仅用于展示与前端路由）
    pub role: Role,
    /// 用户拥有的全部角色标识（来自 user_roles 表，权限判定依据）
    pub roles: Vec<String>,
    /// 是否激活
    pub is_active: bool,
    /// 创建时间
    pub created_at: DateTime<Utc>,
}

impl UserInfo {
    /// 由用户实体与角色列表构造对外信息
    ///
    /// 自动过滤密码哈希；`role` 由 `roles` 推导，二者不会互相矛盾。
    pub fn new(user: User, roles: Vec<String>) -> Self {
        Self {
            id: user.id,
            username: user.username,
            email: user.email,
            role: Role::primary_from(&roles),
            roles,
            is_active: user.is_active,
            created_at: user.created_at,
        }
    }
}
