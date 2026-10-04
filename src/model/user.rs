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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
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
    /// 是否必须先改密才能正常使用系统
    ///
    /// v0.11.0 新增。管理员建号/重置口令时置 `true`，
    /// 用户自助改密后清零。**存量用户一律 `false`**。
    pub must_change_password: bool,
    /// 展示名（可空，存量用户为 NULL）
    ///
    /// v0.20.0 新增。**不是登录键**，与 `username` 不同：
    /// 两个人同名完全合法，管理员在列表里靠 `username` 与 `id` 辨认。
    /// 空表示"没设过"，前端回退显示 `username`——两种状态刻意可区分。
    pub display_name: Option<String>,
    /// 头像相对路径（可空，形如 `/uploads/xxx.png`）
    ///
    /// v0.20.0 新增。DB 层 CHECK 只允许站内 `/uploads/` 前缀，
    /// 见迁移 014 注释里"为什么不给它开外链"的理由。
    pub avatar_url: Option<String>,
    /// 口令最近一次被设置的时刻（v0.22.0）
    ///
    /// 口令过期策略的判据。**不能用 `updated_at` 顶替**：
    /// v0.20.0 的自助改资料、管理员改显示名都会刷新 `updated_at`，
    /// 于是"改了个头像"会被算成"刚换过口令"，过期策略被无限推迟——
    /// 一个安全策略被另一个无关功能静默关掉。
    pub password_changed_at: Option<DateTime<Utc>>,
    /// 创建时间
    pub created_at: DateTime<Utc>,
    /// 更新时间
    pub updated_at: DateTime<Utc>,
}

/// 用户注册请求体
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct RegisterRequest {
    /// 用户名（3-50 个字符，字母/数字/下划线/连字符）
    //
    // 注意：`///` 会被 `utoipa` 导出成**对外**的 OpenAPI 描述，
    // 所以这里只留调用方真正需要的规则，开发过程的话不能写进来。
    //
    // v0.18.0 补上了字符集：原先只写"3-50 个字符"，读起来像是任意字符都能用，
    // 而实际字符集更窄。这不是新增限制，只是让文档说实话。
    pub username: String,
    /// 电子邮箱（含 @ 与 .，最长 255 个字符）
    pub email: String,
    /// 密码（8-128 个字符，且至少含大写/小写/数字/符号中的两类）
    //
    // v0.11.0 把策略从"至少 6 个字符"收紧后，这条注释一直没跟着改，
    // 于是 Swagger 页告诉调用方"6 个字符就够了"——按它写出来的调用方
    // 会被真实接口以 400 拒掉。一份对外文档里的过期规则，
    // 和页面上的 `min: 6` 是同一种缺陷。
    pub password: String,
}

/// 用户登录请求体
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct LoginRequest {
    /// 用户名或邮箱
    pub username: String,
    /// 密码
    pub password: String,
}

/// 登录成功响应
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct LoginResponse {
    /// JWT 访问令牌
    pub token: String,
    /// 令牌类型
    pub token_type: String,
    /// 令牌是否为"受限令牌"（用户须先改密）
    ///
    /// v0.11.0 新增。前端据此跳转到强制改密页，
    /// **但真正的拦截在后端 `auth_middleware`**——
    /// 只靠前端跳转就等于把权限校验交给界面，
    /// 与 v0.10.0 关掉的"界面替后端承诺"是同一类错误。
    pub must_change_password: bool,
}

/// 自助修改密码请求体
///
/// `deny_unknown_fields`：本端点只能改口令。多传一个 `is_active` 或 `roles`
/// 绝不能被静默忽略——那会让调用方以为"改了"，实际什么都没发生。
#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangePasswordRequest {
    /// 当前口令
    pub old_password: String,
    /// 新口令（需满足复杂度策略）
    pub new_password: String,
}

/// 自助修改资料请求体
///
/// `deny_unknown_fields`：与 [`ChangePasswordRequest`] 同理。
/// 这个端点**只能**改展示型字段。多传一个 `email`、`is_active` 或 `roles`
/// 若被静默忽略，调用方会以为"改了"，实际什么都没发生——
/// 而"以为改了邮箱"这种误解在找回账号时才会暴露，那时已经晚了。
///
/// 刻意**不含 `email`**：没有邮件通道就无法证明新邮箱归提交者所有，
/// 放开它等于把账号找回凭据交给任何登录用户。理由见迁移 014 注释。
#[derive(Debug, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateProfileRequest {
    /// 展示名（1-50 个字符，可为 null 清空）
    ///
    /// 为 null 表示"清空并回退显示用户名"。空串按清空处理，
    /// 不用 null 与空串表达同一个意思——那正是迁移 014 拒绝的形态二义性。
    #[serde(
        default,
        deserialize_with = "crate::utils::validation::deserialize_display_name"
    )]
    pub display_name: Option<Option<String>>,
    /// 头像相对路径（可为 null 清空）
    ///
    /// 只接受 `/uploads/` 开头的站内相对路径；DB 层 CHECK 会再挡一道。
    #[serde(
        default,
        deserialize_with = "crate::utils::validation::deserialize_avatar_url"
    )]
    pub avatar_url: Option<Option<String>>,
}

/// 当前用户信息（对外暴露，不含密码）
#[derive(Debug, Serialize, utoipa::ToSchema)]
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
    /// 是否必须先改密（前端据此强制跳转改密页）
    pub must_change_password: bool,
    /// 展示名；未设置时为 NULL，由前端回退显示用户名
    pub display_name: Option<String>,
    /// 头像相对路径；未设置时为 NULL
    pub avatar_url: Option<String>,
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
            must_change_password: user.must_change_password,
            display_name: user.display_name,
            avatar_url: user.avatar_url,
            created_at: user.created_at,
        }
    }

    /// 展示用名称：未设 `display_name` 时回退到用户名
    ///
    /// 单独给方法而不是让前端各写一遍 `display_name || username`：
    /// 这个回退规则一旦在前端出现第二份，两处迟早会不一致，
    /// 而不一致的表现是"有的地方显示空白"——很难被测出来。
    pub fn display_label(&self) -> &str {
        self.display_name
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(&self.username)
    }
}
