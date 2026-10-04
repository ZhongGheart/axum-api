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
use crate::service::setting::SettingService;
use crate::utils::jwt::JwtUtil;
use crate::utils::password::{check_password, hash_password, PasswordCheck};
use crate::utils::redis::RedisClient;
use crate::utils::validation;

/// 管理员解锁的结果（对外返回，让"解锁了什么"可核对）
///
/// 刻意回**清掉了多少次失败计数**而不只是 "ok"：
/// 管理员需要区分"确实有锁定并已解除"与"本来就没锁，点了没反应"——
/// 这两种在只回 ok 的接口里长得一模一样。
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct UnlockedAccount {
    /// 被解锁的用户名
    pub username: String,
    /// 清除的登录失败计数总次数（0 表示本来就没有锁定）
    pub cleared_failures: u64,
    /// 实际写入过的计数桶数量（最多 2：用户名与邮箱各一）
    pub scopes_cleared: usize,
}

/// 对外返回的单条会话（比 Redis 里的记录多一个"这是不是你当前这条"）
///
/// `is_current` 让前端能标出"当前设备"并**禁止吊销它自己**：
/// 吊销当前令牌会让这次请求的下一次调用立刻 401，用户看到的是
/// "系统把我踢了"，而不是"你刚才点了下线自己的设备"。差异很真实。
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct SessionView {
    /// 令牌唯一标识（吊销时回传）
    pub jti: String,
    /// 登录时刻（Unix 毫秒）
    pub login_at_ms: i64,
    /// 令牌到期时刻（Unix 毫秒）
    pub expires_at_ms: i64,
    /// 客户端 IP
    pub client_ip: String,
    /// 是否为发起本次请求的会话
    pub is_current: bool,
}

/// 会话吊销结果
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct RevokedSession {
    /// 被吊销的令牌 jti
    pub jti: String,
    /// 该令牌在登记时的到期时刻（Unix 毫秒）
    pub expires_at_ms: i64,
    /// 剩余会话数
    pub remaining_sessions: usize,
}

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
    /// 系统参数服务（v0.22.0）：登录失败阈值与窗口从这里**动态**读
    pub setting_service: SettingService,
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
        audit_repo: AuditLogRepository,
        setting_service: SettingService,
    ) -> Self {
        Self {
            user_repo,
            role_repo,
            jwt_util,
            jwt_expiration_seconds,
            setting_service,
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
        // 注册准入判定放在**所有校验之前**：关闭注册时不该因为
        // "用户名不合法"而返回 400——那会泄露"这个用户名能不能用"，
        // 也会让前端把关闭注册显示成一次表单校验失败。
        //
        // 失败也落审计：关闭注册后仍有人来撞注册口，
        // 与登录失败计数一样，是"有人在试探"的直接证据。
        if !self.setting_service.registration_enabled().await {
            self.audit(AuthAudit {
                action: Self::ACTION_REGISTER,
                path: Self::PATH_REGISTER,
                status_code: 403,
                client_ip,
                username: &req.username,
                user_id: None,
                result: "注册已关闭",
            })
            .await?;
            return Err(AppError::Forbidden);
        }

        // 归一在**校验之前**：`users.username` / `users.email` 的唯一数据源。
        // 下面的查重用的就是归一后的值，于是查重自动变成"归一后比较"——
        // 不必再单独写一条大小写不敏感的查重，那样两处规则迟早会走偏。
        let username = validation::normalize_username(&req.username)?;
        // 策略来自参数表（v0.22.0），管理员改完立即生效
        let policy = self.setting_service.password_policy().await;
        validation::validate_password_with(&req.password, &policy)?;
        let email = validation::normalize_email(&req.email)?;

        if (self.user_repo.find_by_username(&username).await?).is_some() {
            self.audit(AuthAudit {
                action: Self::ACTION_REGISTER,
                path: Self::PATH_REGISTER,
                status_code: 409,
                client_ip,
                username: &username,
                user_id: None,
                result: "用户名已被注册",
            })
            .await?;
            return Err(AppError::Conflict("用户名已被注册".to_string()));
        }

        if (self.user_repo.find_by_email(&email).await?).is_some() {
            self.audit(AuthAudit {
                action: Self::ACTION_REGISTER,
                path: Self::PATH_REGISTER,
                status_code: 409,
                client_ip,
                username: &username,
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
                &username,
                &email,
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
            // 记**归一后**的值，与本函数其余几条审计一致：
            // 有人拿 `Admin` 去撞已存在的 `admin` 时，审计里留下的是
            // 一次针对真账号的 409，而不是一个看不出意图的 `Admin`
            username: &username,
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
        // 归一一次，下面全流程复用：查库、限流 key、审计三处必须**同一个值**。
        // 分头各算一次的话，哪天有人给其中一处忘了归一，
        // 就会得到"限流按小写计数、查库按原样查"这种对不上的账。
        let login_input = validation::normalize_login_input(&req.username);
        let account_scope = format!("account:{login_input}");
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
                    username: &login_input,
                    user_id: None,
                    result: "失败次数过多，账号或来源 IP 已锁定",
                })
                .await?;
            }
            return Err(e);
        }

        // 登录输入同样要归一：`admin` / `Admin` / `ADMIN@Example.com`
        // 必须落到同一个账号，否则"大小写不敏感"只是写入侧的一半承诺
        let user = match self
            .user_repo
            .find_by_username_or_email(&login_input)
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
                    username: &login_input,
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
                username: &login_input,
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
                    username: &login_input,
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

        // ── 口令过期判定（v0.22.0）────────────────────────────────
        //
        // **过期不影响登录本身**，只是把令牌降级成 v0.11.0 那套"受限令牌"。
        //
        // 刻意复用已有的受限令牌机制而不是新增一种拒绝：
        // 拒绝会让过期用户**直接登不进**，而他们已经无法自助恢复
        // （本仓没有邮件通道，改邮箱也验证不了归属）。
        // 那等于把"口令过期"变成"账号永久锁死，且只能找管理员重置"。
        // 降级令牌则给出一条确定能走完的路：登录 → 改密 → 令牌恢复完整。
        //
        // 与 `must_change_password` 取或：管理员重置的口令本来就要求改，
        // 而过期是另一条独立原因，两者都不该互相覆盖。
        let policy = self.setting_service.password_policy().await;
        let expired = policy.is_expired(user.password_changed_at);
        let must_change_password = user.must_change_password || expired;

        let (token, token_jti) = self
            .jwt_util
            .sign_with_jti(
                user.id,
                &user.username,
                &primary_role.to_string(),
                &roles,
                self.jwt_expiration_seconds,
                must_change_password,
            )
            .map_err(|e| AppError::InternalServerError(format!("JWT 签发失败: {e}")))?;

        self.clear_login_failures(redis_client, &account_scope, &ip_scope)
            .await;

        // 登记会话（"谁在线"的唯一数据来源）。
        //
        // **写失败不放行登录**：登记只用于管理视图，缺一条记录不影响认证结论，
        // 但它会让"这个人在哪些设备登录"漏掉一次登录——而管理员正是靠这个列表
        // 判断账号是否被盗用。让一次失败变成"那次登录对管理员不可见"，
        // 比让整个登录失败更危险：后者用户会知道并重试，前者用户毫无察觉。
        //
        // TTL 等于令牌寿命，令牌失效时这条记录由 Redis 自动清理。
        let now_ms = chrono::Utc::now().timestamp_millis();
        let record = crate::utils::redis::SessionRecord {
            jti: token_jti.clone(),
            user_id: user.id,
            username: user.username.clone(),
            client_ip: client_ip.to_string(),
            login_at_ms: now_ms,
            expires_at_ms: now_ms + (self.jwt_expiration_seconds as i64) * 1000,
        };
        redis_client
            .register_session(&record, self.jwt_expiration_seconds)
            .await?;

        self.audit(AuthAudit {
            action: Self::ACTION_LOGIN_SUCCESS,
            path: Self::PATH_LOGIN,
            status_code: 200,
            client_ip,
            username: &user.username,
            user_id: Some(user.id),
            result: match (user.must_change_password, expired) {
                (true, true) => "登录成功（受限令牌：管理员要求改密 + 口令已过期）",
                (true, false) => "登录成功（受限令牌：待改密）",
                (false, true) => "登录成功（受限令牌：口令已过期）",
                (false, false) => "登录成功",
            },
        })
        .await?;

        tracing::info!("用户登录成功: {}", user.username);

        Ok(LoginResponse {
            token,
            token_type: "Bearer".to_string(),
            must_change_password,
        })
    }

    /// 用户登出：只注销当前令牌（jti）
    pub async fn logout(
        &self,
        redis_client: &RedisClient,
        user_id: uuid::Uuid,
        token_jti: &str,
        token_exp: u64,
    ) -> Result<(), AppError> {
        redis_client
            .add_token_to_blacklist(token_jti, token_exp)
            .await?;

        // 除黑名单外还要删会话登记——否则"谁在线"会把一个已经登出的会话
        // 继续显示成在线，且要等到令牌自然过期才消失。
        //
        // 删除失败**不**让登出失败：令牌此刻已在黑名单里、事实上已作废，
        // 此时返回错误会让用户以为"我没登出成功"而去重试，
        // 而重试时令牌已失效，只会得到一个更难懂的 401。
        // 所以只记警告，让列表残留至多存在到令牌过期。
        if let Err(e) = redis_client.remove_session(user_id, token_jti).await {
            tracing::warn!("会话登记删除失败（令牌已注销，仅在线列表会残留）: {e}");
        }

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
    ///
    /// 管理员解锁账号（清除登录爆破防护的失败计数）
    ///
    /// ── 为什么必须同时清 username 与 email 两个 scope ──
    /// 登录失败计数写在 `account:{归一后的登录输入}` 下（见 `login` 里的
    /// `format!("account:{login_input}")`），而登录框**用户名和邮箱都接受**。
    /// 于是同一个账号有两个独立的计数桶：`account:alice` 与
    /// `account:alice@example.com`。攻击者用哪种标识都能把账号锁上。
    ///
    /// 只清其中一个的话：**通过另一条路径锁定的用户仍然登不进去**，
    /// 而管理员看到"已解锁"却仍被拒。这不是边角情况——两种标识
    /// 分别填在登录框和另一个工具里是常态。
    ///
    /// ── 为什么绝不碰 `ip:` scope ──
    /// IP 计数是**跨账号共享**的。一个 NAT 出口后面的所有用户共用一个桶。
    /// 管理员解锁 alice 时把 IP 桶一起清零，等于给正在爆破的地址发了一份
    /// 新额度——不只是 alice 没被救，还顺手帮了攻击者。
    /// 所以这里只清账号维度，IP 维度交给它自己的 TTL 自然过期。
    pub async fn unlock_user(
        &self,
        redis_client: &RedisClient,
        user: &crate::model::User,
    ) -> Result<UnlockedAccount, AppError> {
        // 与 login 走**同一个**归一函数，否则清掉的 key 与写入的 key 对不上。
        // 这里直接复用库里已归一的 username / email（v0.19.0 之后写入必为小写无空白）。
        let scopes = [
            format!("account:{}", user.username),
            format!("account:{}", user.email),
        ];

        let mut cleared = 0u64;
        let mut remaining = Vec::new();
        for scope in &scopes {
            let before = redis_client.login_failure_count(scope).await?;
            if let Err(e) = redis_client.clear_login_failures(scope).await {
                // 清失败不改变"是否成功"的结论，但必须可观测且**不能静默成功**：
                // 若 Redis 挂了而我们回 200，管理员会以为解锁了而用户仍登不进去。
                tracing::error!("解锁失败：清除 {scope} 出错: {e}");
                return Err(AppError::InternalServerError(format!(
                    "解锁失败：Redis 操作出错: {e}"
                )));
            }
            if before > 0 {
                cleared += before;
                remaining.push((scope.clone(), before));
            }
        }

        tracing::info!(
            target: "service",
            "管理员解锁账号: username={}, 清除失败计数 {} 次, 涉及 {} 个桶",
            user.username,
            cleared,
            remaining.len()
        );

        Ok(UnlockedAccount {
            username: user.username.clone(),
            cleared_failures: cleared,
            scopes_cleared: remaining.len(),
        })
    }

    /// 列出某用户的在线会话（管理端）
    pub async fn list_sessions(
        &self,
        redis_client: &RedisClient,
        user_id: uuid::Uuid,
        current_jti: &str,
    ) -> Result<Vec<SessionView>, AppError> {
        let records = redis_client.list_sessions(user_id).await?;
        Ok(records
            .into_iter()
            .map(|r| SessionView {
                is_current: r.jti == current_jti,
                jti: r.jti,
                login_at_ms: r.login_at_ms,
                expires_at_ms: r.expires_at_ms,
                client_ip: r.client_ip,
            })
            .collect())
    }

    /// 吊销**单个**会话
    ///
    /// ── 为什么单会话吊销必须走 jti 黑名单，不能走 `revoke_user_sessions` ──
    /// 后者是**整用户粒度**的时间戳（`user_revoked_before`）：它一次作废该用户
    /// 所有在那个时刻之前签发的令牌。要踢掉一台设备却把这个人所有设备都踢了，
    /// 那不是"单会话吊销"，是换个名字的全量重登。
    ///
    /// 查不到会话登记时**明确拒绝**（404）而不是回成功：登记可能因故丢失，
    /// 而"界面显示已下线、令牌其实还在有效期内"是一个**看起来完全正常**的
    /// 安全假象——管理员会据此认为风险已排除。
    pub async fn revoke_session(
        &self,
        redis_client: &RedisClient,
        user_id: uuid::Uuid,
        jti: &str,
    ) -> Result<RevokedSession, AppError> {
        let record = redis_client
            .get_session(user_id, jti)
            .await?
            .ok_or_else(|| {
                AppError::NotFound("会话不存在或已过期（可能已登出或令牌已失效）".into())
            })?;

        // 黑名单 TTL 用令牌**剩余**寿命而不是配置值：
        // 用配置值会让一条本该只再活 5 分钟的令牌在黑名单里躺满一整天，
        // 而黑名单键是按 jti 存的——无意义地占着内存。
        let now_ms = chrono::Utc::now().timestamp_millis();
        let remaining = ((record.expires_at_ms - now_ms).max(0) as u64) / 1000 + 1;

        redis_client
            .add_token_to_blacklist(&record.jti, remaining)
            .await?;
        redis_client.remove_session(user_id, jti).await?;

        let remaining_sessions = redis_client.list_sessions(user_id).await?.len();

        tracing::info!(
            target: "service",
            "吊销单个会话: user_id={user_id}, 剩余会话={remaining_sessions}"
        );

        Ok(RevokedSession {
            jti: record.jti,
            expires_at_ms: record.expires_at_ms,
            remaining_sessions,
        })
    }

    /// 自助修改资料（仅展示型字段）
    ///
    /// **不吊销会话**：改展示名与头像不影响凭据，
    /// 吊销会把用户正在用的页面直接踢回登录页——而他并没有做任何需要重新
    /// 确认身份的事。对照 [`Self::change_password`] 改完必须吊销：
    /// 那一次动的是口令本身。
    pub async fn update_profile(
        &self,
        user_id: uuid::Uuid,
        display_name: Option<Option<&str>>,
        avatar_url: Option<Option<&str>>,
    ) -> Result<crate::model::UserInfo, AppError> {
        let user = self
            .user_repo
            .update_profile(user_id, display_name, avatar_url)
            .await?;

        // 返回**改完之后**的完整信息，而不是只回一个 "ok"：
        // 调用方需要立刻拿到归一后的值去更新界面，
        // 若让它自己乐观地改本地状态，与服务端不一致时（比如并发被别人改了）
        // 界面就会一直显示一个库里没有的名字。
        let roles = self.role_repo.find_roles_by_user_id(user_id).await?;
        tracing::info!(
            target: "service",
            "用户自助修改资料: user_id={}, display_name={}, 头像={}",
            user_id,
            display_name.flatten().map(|s| if s.is_empty() { "(清空)" } else { s }).unwrap_or("(未提供)"),
            if avatar_url.flatten().is_some() { "已设置" } else { "未变" }
        );
        Ok(crate::model::UserInfo::new(user, roles))
    }

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
        // 策略来自参数表（v0.22.0）。
        //
        // 注意这条路径**只在设置口令时**校验复杂度。登录校验的是 Argon2 哈希，
        // 与策略无关——所以管理员抬高门槛不会把存量用户锁在门外。
        let policy = self.setting_service.password_policy().await;
        validation::validate_password_with(new_password, &policy)?;

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

        // 同时清掉会话登记。时间戳机制已经让令牌失效了，但登记还留在 Redis 里，
        // 于是"在线会话"列表会继续把一台台其实已经登不上的设备显示成在线——
        // 直到各自的 TTL 自然过期（默认整整一天）。
        //
        // 清理失败**不回错**：令牌此刻已确实失效，认证结论已经成立。
        // 若因清理失败而让改密/停用报 500，用户会以为密码没改成功而重试，
        // 而此时令牌已吊销，重试只会得到更难解释的 401。
        if let Err(e) = redis_client
            .delete_by_prefix(&format!("sess:{user_id}:"))
            .await
        {
            tracing::warn!("全量吊销后会话登记清理失败（令牌已失效，仅在线列表残留）: {e}");
        }

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
        // 阈值每次登录现读，而不是构造时固化：
        // 固化的话管理员在参数页把阈值从 10 调到 3，
        // 必须重启进程才生效——而"改一个数字要重启"正是
        // 参数表要解决的那个问题。
        let max_failures = self.setting_service.login_max_failures().await;
        let account_failures = redis_client.login_failure_count(account_scope).await?;
        let ip_failures = redis_client.login_failure_count(ip_scope).await?;

        if account_failures >= max_failures || ip_failures >= max_failures {
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
        let window = self.setting_service.login_failure_window_seconds().await;
        for scope in [account_scope, ip_scope] {
            if let Err(e) = redis_client.record_login_failure(scope, window).await {
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
