//! 认证服务层
//!
//! 封装用户注册、登录、获取当前用户等业务逻辑。
//! 调用 Repository 层进行数据访问，调用 Utils 层处理密码和 JWT。
//! RBAC：注册时自动分配默认角色，登录时将角色列表写入 JWT。

use uuid::Uuid;

use crate::error::AppError;
use crate::model::{LoginRequest, LoginResponse, RegisterRequest, Role, UserInfo};
use crate::repository::audit_log::{AuditEntry, AuditLogRepository};
use crate::repository::role::RoleRepository;
use crate::repository::user::UserRepository;
use crate::utils::jwt::JwtUtil;
use crate::utils::password::{check_password, hash_password, PasswordCheck};
use crate::utils::redis::RedisClient;
use crate::utils::validation;

/// 认证服务
///
/// 组合 Repository 和 JWT 工具，提供完整的认证业务逻辑。
#[derive(Debug, Clone)]
pub struct AuthService {
    /// 用户仓储
    pub user_repo: UserRepository,
    /// 角色仓储（用于 RBAC 角色查询与分配）
    pub role_repo: RoleRepository,
    /// JWT 工具
    pub jwt_util: JwtUtil,
    /// JWT 过期时间（秒）
    pub jwt_expiration_seconds: u64,
    /// 登录失败锁定阈值（账号维度与 IP 维度各自独立计数）
    pub login_max_failures: u64,
    /// 登录失败计数窗口（秒）
    pub login_failure_window_seconds: u64,
    /// 审计仓储（登录/注册审计；见 [`Self::login`]）
    pub audit_repo: AuditLogRepository,
}

/// 一条待写的登录/注册审计
///
/// 用结构体而非一串位置参数：`audit(action, path, code, ip, user, id, result)`
/// 七个参数里有两个 `Option`，编译器**不会**提醒调用方把 `None` 放错了位置。
#[derive(Debug, Clone, Copy)]
pub struct AuthAudit<'a> {
    /// 动作标识
    pub action: &'a str,
    /// 请求路径
    pub path: &'a str,
    /// HTTP 状态码
    pub status_code: u16,
    /// 客户端 IP
    pub client_ip: &'a str,
    /// 用户名。登录失败时记**尝试登录时提交的名字**，即使查无此人
    pub username: &'a str,
    /// 已认证用户 ID；登录失败等场景为 `None`
    pub user_id: Option<uuid::Uuid>,
    /// 结果说明
    pub result: &'a str,
}

impl AuthService {
    /// 登录路径（审计记录的 `path` 列）
    const PATH_LOGIN: &'static str = "/api/auth/login";
    /// 注册路径（审计记录的 `path` 列）
    const PATH_REGISTER: &'static str = "/api/auth/register";

    /// 登录成功
    ///
    /// 刻意**不用**中间件那套 `"{METHOD} {path}"` 格式：登录成功与登录失败
    /// 的方法、路径完全相同，用方法+路径就无法区分，
    /// 事后也就无法回答"有没有人在爆破这个账号"。
    pub const ACTION_LOGIN_SUCCESS: &'static str = "AUTH_LOGIN_SUCCESS";
    /// 登录失败（含账号不存在 / 口令不符 / 账号停用 / 已锁定）
    pub const ACTION_LOGIN_FAILURE: &'static str = "AUTH_LOGIN_FAILURE";
    /// 注册成功
    pub const ACTION_REGISTER: &'static str = "AUTH_REGISTER";

    /// 写一条登录/注册审计
    ///
    /// **同步写、写不进去就让请求失败**（与中间件的 `tokio::spawn` 相反）：
    /// 登录是安全关键路径，审计静默丢失等于没审计。
    ///
    /// `username` 记的是**尝试登录时提交的名字**，即使查无此人也要记——
    /// 那正是爆破的证据。写入失败会**覆盖**业务错误返回 500，
    /// 这是有意的取舍：审计不可用时不能假装"这次登录已被记录"。
    async fn audit(&self, a: AuthAudit<'_>) -> Result<(), AppError> {
        let entry = AuditEntry::auth(a.action, "POST", a.path, a.status_code as i32, a.client_ip)
            .with_identity(a.user_id, Some(a.username.to_string()))
            .with_result(a.result);
        self.audit_repo.record(&entry).await
    }

    /// 创建新的 AuthService 实例
    pub fn new(
        user_repo: UserRepository,
        role_repo: RoleRepository,
        jwt_util: JwtUtil,
        jwt_expiration_seconds: u64,
        login_max_failures: u64,
        login_failure_window_seconds: u64,
        audit_repo: AuditLogRepository,
    ) -> Self {
        Self {
            user_repo,
            role_repo,
            jwt_util,
            jwt_expiration_seconds,
            login_max_failures,
            login_failure_window_seconds,
            audit_repo,
        }
    }

    /// 用户注册
    ///
    /// `client_ip` 是新增参数：注册此前**完全不进审计**，
    /// 而"谁在何时从哪注册了一个账号"正是账号滥用的第一条线索。
    pub async fn register(
        &self,
        req: RegisterRequest,
        client_ip: &str,
    ) -> Result<UserInfo, AppError> {
        validation::validate_username(&req.username)?;
        validation::validate_password(&req.password)?;
        validation::validate_email(&req.email)?;

        if (self.user_repo.find_by_username(&req.username).await?).is_some() {
            self.audit(AuthAudit {
                action: Self::ACTION_REGISTER,
                path: Self::PATH_REGISTER,
                status_code: 409,
                client_ip,
                username: &req.username,
                user_id: None,
                result: "用户名已被注册",
            })
            .await?;
            return Err(AppError::Conflict("用户名已被注册".to_string()));
        }

        if (self.user_repo.find_by_email(&req.email).await?).is_some() {
            self.audit(AuthAudit {
                action: Self::ACTION_REGISTER,
                path: Self::PATH_REGISTER,
                status_code: 409,
                client_ip,
                username: &req.username,
                user_id: None,
                result: "邮箱已被注册",
            })
            .await?;
            return Err(AppError::Conflict("邮箱已被注册".to_string()));
        }

        let password_hash = hash_password(&req.password)
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;

        let user = self
            .user_repo
            .create(
                Uuid::new_v4(),
                &req.username,
                &req.email,
                &password_hash,
                // 用户自己设的口令，不需要强制改密
                false,
            )
            .await?;

        self.role_repo.assign_role_to_user(user.id, "user").await?;

        self.audit(AuthAudit {
            action: Self::ACTION_REGISTER,
            path: Self::PATH_REGISTER,
            status_code: 200,
            client_ip,
            username: &req.username,
            user_id: Some(user.id),
            result: "注册成功",
        })
        .await?;

        tracing::info!("新用户注册成功: {} (已分配 user 角色)", user.username);

        Ok(UserInfo::new(user, vec!["user".to_string()]))
    }

    /// 用户登录
    ///
    /// 失败按「账号」与「客户端 IP」双维度计数，超阈值直接拒绝，遏制在线口令爆破。
    /// v0.1 的 `Argon2(sha256(明文))` 存量口令会在登录成功时透明升级为 `Argon2(明文)`。
    ///
    /// **成功与全部失败分支都写审计**。此前登录完全不在 `audit_logs` 里，
    /// 失败只进 Redis 计数器——而计数器带 TTL 会过期，
    /// 过期后就再也答不出"谁在何时从哪尝试过这个账号"。
    pub async fn login(
        &self,
        req: LoginRequest,
        redis_client: &RedisClient,
        client_ip: &str,
    ) -> Result<LoginResponse, AppError> {
        let account_scope = format!("account:{}", req.username.trim().to_lowercase());
        let ip_scope = format!("ip:{client_ip}");

        // 被锁定也要落审计：这正是"有人在爆破"最直接的证据，
        // 只记 Redis 计数器会在窗口过期后彻底消失。
        // 但 Redis 故障导致的错误**不写审计**——那种情况下审计写入
        // 未必可用，硬写只会把一个 503 变成 500。
        if let Err(e) = self
            .ensure_not_locked(redis_client, &account_scope, &ip_scope)
            .await
        {
            if matches!(e, AppError::TooManyRequests(_)) {
                self.audit(AuthAudit {
                    action: Self::ACTION_LOGIN_FAILURE,
                    path: Self::PATH_LOGIN,
                    status_code: 429,
                    client_ip,
                    username: req.username.trim(),
                    user_id: None,
                    result: "失败次数过多，账号或来源 IP 已锁定",
                })
                .await?;
            }
            return Err(e);
        }

        let user = match self
            .user_repo
            .find_by_username_or_email(req.username.trim())
            .await?
        {
            Some(user) => user,
            None => {
                self.record_login_failure(redis_client, &account_scope, &ip_scope)
                    .await;
                self.audit(AuthAudit {
                    action: Self::ACTION_LOGIN_FAILURE,
                    path: Self::PATH_LOGIN,
                    status_code: 401,
                    client_ip,
                    username: req.username.trim(),
                    user_id: None,
                    result: "账号不存在",
                })
                .await?;
                return Err(AppError::InvalidCredentials("用户名或密码错误".to_string()));
            }
        };

        // 停用账号不参与失败计数，直接拒绝
        if !user.is_active {
            self.audit(AuthAudit {
                action: Self::ACTION_LOGIN_FAILURE,
                path: Self::PATH_LOGIN,
                status_code: 403,
                client_ip,
                username: req.username.trim(),
                user_id: Some(user.id),
                result: "账号已停用",
            })
            .await?;
            return Err(AppError::Forbidden);
        }

        match check_password(&req.password, &user.password_hash)
            .map_err(|e| AppError::InternalServerError(e.to_string()))?
        {
            PasswordCheck::Invalid => {
                self.record_login_failure(redis_client, &account_scope, &ip_scope)
                    .await;
                self.audit(AuthAudit {
                    action: Self::ACTION_LOGIN_FAILURE,
                    path: Self::PATH_LOGIN,
                    status_code: 401,
                    client_ip,
                    username: req.username.trim(),
                    user_id: Some(user.id),
                    result: "口令不符",
                })
                .await?;
                return Err(AppError::InvalidCredentials("用户名或密码错误".to_string()));
            }
            PasswordCheck::ValidNeedsUpgrade(new_hash) => {
                // 已通过校验，升级失败不应阻断本次登录
                if let Err(e) = self
                    .user_repo
                    .update_password_hash(user.id, &new_hash)
                    .await
                {
                    tracing::warn!("v0.1 旧口令格式升级失败 (user={}): {e}", user.id);
                } else {
                    tracing::info!("v0.1 旧口令格式已升级 (user={})", user.id);
                }
            }
            PasswordCheck::Valid => {}
        }

        let roles = self.role_repo.find_roles_by_user_id(user.id).await?;

        tracing::debug!("用户 {} 拥有的角色: {:?}", user.username, roles);

        // 主角色由角色集合推导，不再读取冗余的 users.role 列
        let primary_role = Role::primary_from(&roles);

        let token = self
            .jwt_util
            .sign(
                user.id,
                &user.username,
                &primary_role.to_string(),
                &roles,
                self.jwt_expiration_seconds,
                user.must_change_password,
            )
            .map_err(|e| AppError::InternalServerError(format!("JWT 签发失败: {e}")))?;

        self.clear_login_failures(redis_client, &account_scope, &ip_scope)
            .await;

        self.audit(AuthAudit {
            action: Self::ACTION_LOGIN_SUCCESS,
            path: Self::PATH_LOGIN,
            status_code: 200,
            client_ip,
            username: &user.username,
            user_id: Some(user.id),
            result: if user.must_change_password {
                "登录成功（受限令牌：待改密）"
            } else {
                "登录成功"
            },
        })
        .await?;

        tracing::info!("用户登录成功: {}", user.username);

        Ok(LoginResponse {
            token,
            token_type: "Bearer".to_string(),
            must_change_password: user.must_change_password,
        })
    }

    /// 用户登出：只注销当前令牌（jti）
    pub async fn logout(
        &self,
        redis_client: &RedisClient,
        token_jti: &str,
        token_exp: u64,
    ) -> Result<(), AppError> {
        redis_client
            .add_token_to_blacklist(token_jti, token_exp)
            .await?;

        tracing::info!("令牌已注销: jti={token_jti}");
        Ok(())
    }

    /// 自助修改口令
    ///
    /// 三条不能省的校验：
    ///
    /// 1. **必须验旧口令**。只验新口令复杂度是不够的——
    ///    令牌一旦被劫持，攻击者就能把密码改成自己知道的值并永久占据账号。
    ///    验旧口令把"持有令牌"降级为"持有令牌 **且** 知道口令"
    /// 2. **新旧不能相同**。否则改密是空操作，却回了"改密成功"——
    ///    这正是 v0.10.0 关掉的那类"骗人的成功"
    /// 3. 改密后**吊销全部存量会话**：口令已变，继续有效的旧令牌没有理由留着。
    ///    水位用毫秒（`revoke_user_sessions` 内部即 `timestamp_millis`），
    ///    与 `Claims::iat_ms` 对齐，不会误伤改密后重新登录拿到的令牌
    pub async fn change_password(
        &self,
        redis_client: &RedisClient,
        user_id: uuid::Uuid,
        old_password: &str,
        new_password: &str,
    ) -> Result<(), AppError> {
        let user = self.user_repo.find_by_id(user_id).await?;

        // 先校验新口令复杂度，再验旧口令：复杂度不依赖任何数据库读取，
        // 便宜且能避免为一个必然失败的请求去做 Argon2（刻意昂贵）运算
        validation::validate_password(new_password)?;

        match check_password(old_password, &user.password_hash)
            .map_err(|e| AppError::InternalServerError(e.to_string()))?
        {
            PasswordCheck::Valid | PasswordCheck::ValidNeedsUpgrade(_) => {}
            PasswordCheck::Invalid => {
                // 与登录失败区分：这里是"已登录状态下改密"，401 会让前端
                // 误以为会话过期并跳登录页，所以用 400
                return Err(AppError::BadRequest("当前密码不正确".into()));
            }
        }

        if old_password == new_password {
            return Err(AppError::BadRequest("新密码不能与当前密码相同".into()));
        }

        let hashed = crate::utils::password::hash_password(new_password)
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;
        self.user_repo
            .update_password_hash(user_id, &hashed)
            .await?;

        // 口令已由用户本人确认，清掉强制改密标记
        self.user_repo
            .set_must_change_password(user_id, false)
            .await?;

        // 顺序很重要：**先清标记再吊销会话**。
        // 若吊销成功而清标记失败，用户改完密下次登录仍被拦在改密页，
        // 且此时令牌已被吊销——只能靠管理员重置才能脱困。
        self.revoke_all_sessions(redis_client, user_id).await?;

        tracing::info!("用户自助修改口令并吊销全部会话: {}", user.username);
        Ok(())
    }

    /// 吊销某用户的全部会话
    ///
    /// 用于改密、停用、删除账号等"凭据或状态已变化"的场景。
    pub async fn revoke_all_sessions(
        &self,
        redis_client: &RedisClient,
        user_id: Uuid,
    ) -> Result<(), AppError> {
        redis_client
            .revoke_user_sessions(&user_id, self.jwt_expiration_seconds)
            .await?;
        tracing::info!("已吊销用户全部会话: {user_id}");
        Ok(())
    }

    /// 登录前检查是否已被锁定
    async fn ensure_not_locked(
        &self,
        redis_client: &RedisClient,
        account_scope: &str,
        ip_scope: &str,
    ) -> Result<(), AppError> {
        let account_failures = redis_client.login_failure_count(account_scope).await?;
        let ip_failures = redis_client.login_failure_count(ip_scope).await?;

        if account_failures >= self.login_max_failures || ip_failures >= self.login_max_failures {
            tracing::warn!(
                "登录已锁定: account_failures={account_failures}, ip_failures={ip_failures}"
            );
            return Err(AppError::TooManyRequests(
                "登录失败次数过多，请稍后再试".to_string(),
            ));
        }
        Ok(())
    }

    /// 记录一次登录失败（账号与 IP 双维度）
    ///
    /// 计数写入失败不改变认证结论，但必须可观测。
    async fn record_login_failure(
        &self,
        redis_client: &RedisClient,
        account_scope: &str,
        ip_scope: &str,
    ) {
        for scope in [account_scope, ip_scope] {
            if let Err(e) = redis_client
                .record_login_failure(scope, self.login_failure_window_seconds)
                .await
            {
                tracing::warn!("登录失败计数写入失败 ({scope}): {e}");
            }
        }
    }

    /// 登录成功后清零失败计数
    async fn clear_login_failures(
        &self,
        redis_client: &RedisClient,
        account_scope: &str,
        ip_scope: &str,
    ) {
        for scope in [account_scope, ip_scope] {
            if let Err(e) = redis_client.clear_login_failures(scope).await {
                tracing::warn!("登录失败计数清理失败 ({scope}): {e}");
            }
        }
    }
}
