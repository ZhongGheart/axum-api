//! 认证服务层
//!
//! 封装用户注册、登录、获取当前用户等业务逻辑。
//! 调用 Repository 层进行数据访问，调用 Utils 层处理密码和 JWT。

use uuid::Uuid;

use crate::error::AppError;
use crate::model::{LoginRequest, LoginResponse, UserInfo, RegisterRequest};
use crate::repository::user::UserRepository;
use crate::utils::jwt::JwtUtil;
use crate::utils::password::{hash_password, verify_password};

/// 认证服务
///
/// 组合 Repository 和 JWT 工具，提供完整的认证业务逻辑。
#[derive(Debug, Clone)]
pub struct AuthService {
    /// 用户仓储
    pub user_repo: UserRepository,
    /// JWT 工具
    pub jwt_util: JwtUtil,
    /// JWT 过期时间（秒）
    pub jwt_expiration_seconds: u64,
}

impl AuthService {
    /// 创建新的 AuthService 实例
    pub fn new(
        user_repo: UserRepository,
        jwt_util: JwtUtil,
        jwt_expiration_seconds: u64,
    ) -> Self {
        Self {
            user_repo,
            jwt_util,
            jwt_expiration_seconds,
        }
    }

    /// 用户注册
    ///
    /// 1. 校验用户名和邮箱是否已存在
    /// 2. 对密码进行 Argon2 哈希
    /// 3. 创建用户记录
    /// 4. 返回用户信息（不含密码）
    pub async fn register(&self, req: RegisterRequest) -> Result<UserInfo, AppError> {
        // 参数基本校验
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

        // 检查用户名是否已被注册
        if (self
            .user_repo
            .find_by_username(&req.username)
            .await?)
            .is_some()
        {
            return Err(AppError::Conflict("用户名已被注册".to_string()));
        }

        // 检查邮箱是否已被注册
        if (self.user_repo.find_by_email(&req.email).await?).is_some() {
            return Err(AppError::Conflict("邮箱已被注册".to_string()));
        }

        // 对密码进行 Argon2 哈希
        let password_hash =
            hash_password(&req.password).map_err(|e| AppError::InternalServerError(e.to_string()))?;

        // 创建用户
        let user = self
            .user_repo
            .create(
                Uuid::new_v4(),
                &req.username,
                &req.email,
                &password_hash,
            )
            .await?;

        tracing::info!("新用户注册成功: {}", user.username);

        Ok(UserInfo::from(user))
    }

    /// 用户登录
    ///
    /// 1. 通过用户名/邮箱查找用户
    /// 2. 验证密码
    /// 3. 签发 JWT 令牌
    /// 4. 返回令牌
    pub async fn login(&self, req: LoginRequest) -> Result<LoginResponse, AppError> {
        // 查找用户（支持用户名或邮箱登录）
        let user = self
            .user_repo
            .find_by_username_or_email(&req.username)
            .await?
            .ok_or_else(|| AppError::ValidationFailed("用户名或密码错误".to_string()))?;

        // 检查用户是否激活
        if !user.is_active {
            return Err(AppError::Forbidden);
        }

        // 验证密码
        let is_valid = verify_password(&req.password, &user.password_hash)
            .map_err(|e| AppError::InternalServerError(e.to_string()))?;

        if !is_valid {
            return Err(AppError::ValidationFailed("用户名或密码错误".to_string()));
        }

        // 签发 JWT 令牌
        let token = self
            .jwt_util
            .sign(user.id, &user.role.to_string(), self.jwt_expiration_seconds)
            .map_err(|e| AppError::InternalServerError(format!("JWT 签发失败: {e}")))?;

        tracing::info!("用户登录成功: {}", user.username);

        Ok(LoginResponse {
            token,
            token_type: "Bearer".to_string(),
        })
    }

    /// 获取当前用户信息
    pub async fn get_current_user(&self, user_id: Uuid) -> Result<UserInfo, AppError> {
        let user = self.user_repo.find_by_id(user_id).await?;
        Ok(UserInfo::from(user))
    }
}
