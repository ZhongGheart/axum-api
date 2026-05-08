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
use crate::utils::password::{hash_password, verify_password};
use crate::utils::redis::RedisClient;

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
}

impl AuthService {
    /// 创建新的 AuthService 实例
    pub fn new(
        user_repo: UserRepository,
        role_repo: RoleRepository,
        jwt_util: JwtUtil,
        jwt_expiration_seconds: u64,
    ) -> Self {
        Self {
            user_repo,
            role_repo,
            jwt_util,
            jwt_expiration_seconds,
        }
    }

    /// 用户注册
    pub async fn register(&self, req: RegisterRequest) -> Result<UserInfo, AppError> {
        if req.username.len() < 3 || req.username.len() > 50 {
            return Err(AppError::BadRequest(
                "用户名长度必须在 3-50 个字符之间".to_string(),
            ));
        }
        if req.password.len() < 6 {
            return Err(AppError::BadRequest(
                "密码长度不能少于 6 个字符".to_string(),
            ));
        }
        if !req.email.contains('@') {
            return Err(AppError::BadRequest("邮箱格式不正确".to_string()));
        }

        if (self
            .user_repo
            .find_by_username(&req.username)
            .await?)
            .is_some()
        {
            return Err(AppError::Conflict("用户名已被注册".to_string()));
        }

        if (self.user_repo.find_by_email(&req.email).await?).is_some() {
            return Err(AppError::Conflict("邮箱已被注册".to_string()));
        }

        let password_hash =
            hash_password(&req.password).map_err(|e| AppError::InternalServerError(e.to_string()))?;

        let user = self
            .user_repo
            .create(
                Uuid::new_v4(),
                &req.username,
                &req.email,
                &password_hash,
            )
            .await?;

        self.role_repo
            .assign_role_to_user(user.id, "user")
            .await?;

        tracing::info!("新用户注册成功: {} (已分配 user 角色)", user.username);

        Ok(UserInfo::from(user))
    }

    /// 用户登录（成功后清除 Redis 黑名单，避免旧登出记录阻塞新 token）
    pub async fn login(&self, req: LoginRequest, redis_client: &RedisClient) -> Result<LoginResponse, AppError> {
        let user = self
            .user_repo
            .find_by_username_or_email(&req.username)
            .await?
            .ok_or_else(|| AppError::ValidationFailed("用户名或密码错误".to_string()))?;

        if !user.is_active {
            return Err(AppError::Forbidden);
        }

        let is_valid = verify_password(&req.password, &user.password_hash)
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;

        if !is_valid {
            return Err(AppError::ValidationFailed("用户名或密码错误".to_string()));
        }

        let roles = self
            .role_repo
            .find_roles_by_user_id(user.id)
            .await?;

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

        // 清除 Redis 黑名单（如果存在旧登出记录，避免新 token 被阻塞）
        let _ = redis_client
            .remove_token_blacklist(&user.id.to_string())
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

    /// 用户登出：将 Token 加入 Redis 黑名单
    pub async fn logout(
        &self,
        redis_client: &RedisClient,
        user_id: Uuid,
        token_exp: u64,
    ) -> Result<(), AppError> {
        redis_client
            .add_token_to_blacklist(&user_id.to_string(), token_exp)
            .await?;

        tracing::info!("用户登出成功: {}", user_id);
        Ok(())
    }
}
