//! 认证服务层
//!
//! 封装用户注册、登录、获取当前用户等业务逻辑。
//! 调用 Repository 层进行数据访问，调用 Utils 层处理密码和 JWT。
//! RBAC：注册时自动分配默认角色，登录时将角色列表写入 JWT。

use uuid::Uuid;

use crate::error::AppError;
use crate::model::{LoginRequest, LoginResponse, RegisterRequest, UserInfo};
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
}

impl AuthService {
    /// 创建新的 AuthService 实例
    pub fn new(
        user_repo: UserRepository,
        role_repo: RoleRepository,
        jwt_util: JwtUtil,
        jwt_expiration_seconds: u64,
        login_max_failures: u64,
        login_failure_window_seconds: u64,
    ) -> Self {
        Self {
            user_repo,
            role_repo,
            jwt_util,
            jwt_expiration_seconds,
            login_max_failures,
            login_failure_window_seconds,
        }
    }

    /// 用户注册
    pub async fn register(&self, req: RegisterRequest) -> Result<UserInfo, AppError> {
        validation::validate_username(&req.username)?;
        validation::validate_password(&req.password)?;
        validation::validate_email(&req.email)?;

        if (self.user_repo.find_by_username(&req.username).await?).is_some() {
            return Err(AppError::Conflict("用户名已被注册".to_string()));
        }

        if (self.user_repo.find_by_email(&req.email).await?).is_some() {
            return Err(AppError::Conflict("邮箱已被注册".to_string()));
        }

        let password_hash = hash_password(&req.password)
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;

        let user = self
            .user_repo
            .create(Uuid::new_v4(), &req.username, &req.email, &password_hash)
            .await?;

        self.role_repo
            .assign_role_to_user(user.id, "user")
            .await?;

        tracing::info!("新用户注册成功: {} (已分配 user 角色)", user.username);

        Ok(UserInfo::from(user))
    }

    /// 用户登录
    ///
    /// 失败按「账号」与「客户端 IP」双维度计数，超阈值直接拒绝，遏制在线口令爆破。
    /// v0.1 的 `Argon2(sha256(明文))` 存量口令会在登录成功时透明升级为 `Argon2(明文)`。
    pub async fn login(
        &self,
        req: LoginRequest,
        redis_client: &RedisClient,
        client_ip: &str,
    ) -> Result<LoginResponse, AppError> {
        let account_scope = format!("account:{}", req.username.trim().to_lowercase());
        let ip_scope = format!("ip:{client_ip}");

        self.ensure_not_locked(redis_client, &account_scope, &ip_scope)
            .await?;

        let user = match self
            .user_repo
            .find_by_username_or_email(req.username.trim())
            .await?
        {
            Some(user) => user,
            None => {
                self.record_login_failure(redis_client, &account_scope, &ip_scope)
                    .await;
                return Err(AppError::InvalidCredentials("用户名或密码错误".to_string()));
            }
        };

        // 停用账号不参与失败计数，直接拒绝
        if !user.is_active {
            return Err(AppError::Forbidden);
        }

        match check_password(&req.password, &user.password_hash)
            .map_err(|e| AppError::InternalServerError(e.to_string()))?
        {
            PasswordCheck::Invalid => {
                self.record_login_failure(redis_client, &account_scope, &ip_scope)
                    .await;
                return Err(AppError::InvalidCredentials("用户名或密码错误".to_string()));
            }
            PasswordCheck::ValidNeedsUpgrade(new_hash) => {
                // 已通过校验，升级失败不应阻断本次登录
                if let Err(e) = self.user_repo.update_password_hash(user.id, &new_hash).await {
                    tracing::warn!("v0.1 旧口令格式升级失败 (user={}): {e}", user.id);
                } else {
                    tracing::info!("v0.1 旧口令格式已升级 (user={})", user.id);
                }
            }
            PasswordCheck::Valid => {}
        }

        let roles = self.role_repo.find_roles_by_user_id(user.id).await?;

        tracing::debug!("用户 {} 拥有的角色: {:?}", user.username, roles);

        let token = self
            .jwt_util
            .sign(
                user.id,
                &user.role.to_string(),
                &roles,
                self.jwt_expiration_seconds,
            )
            .map_err(|e| AppError::InternalServerError(format!("JWT 签发失败: {e}")))?;

        self.clear_login_failures(redis_client, &account_scope, &ip_scope)
            .await;

        tracing::info!("用户登录成功: {}", user.username);

        Ok(LoginResponse {
            token,
            token_type: "Bearer".to_string(),
        })
    }

    /// 获取当前用户信息
    #[allow(dead_code)]
    pub async fn get_current_user(&self, user_id: Uuid) -> Result<UserInfo, AppError> {
        let user = self.user_repo.find_by_id(user_id).await?;
        Ok(UserInfo::from(user))
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

        if account_failures >= self.login_max_failures
            || ip_failures >= self.login_max_failures
        {
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
