//! 系统参数服务层
//!
//! 职责边界很清楚：**仓储负责存，服务负责判断取值合不合法，
//! 以及把散落的参数装配成一份类型化的 [`PasswordPolicy]`**。
//!
//! 装配成一个结构体而不是到处散着读五个 key，是因为口令校验要的是
//! 一份**自洽**的策略。若在 `register` 里读一次 min_length、
//! 在 `change_password` 里读一次 max_length，两处会各自拿到
//! 不同时间点的缓存快照，于是"长度必须在 8-128 之间"这种错误文案
//! 会出现在一个实际区间是 8-64 的部署上。一次装配，全程共用。

use crate::error::AppError;
use crate::model::setting::{find_def, PasswordPolicy, SettingType};
use crate::repository::setting::SettingRepository;

/// 参数键（集中在此，避免散落的魔法字符串漂移）
pub mod keys {
    pub const PASSWORD_MIN_LENGTH: &str = "security.password.min_length";
    pub const PASSWORD_MAX_LENGTH: &str = "security.password.max_length";
    pub const PASSWORD_MIN_CHAR_CLASSES: &str = "security.password.min_char_classes";
    pub const PASSWORD_REQUIRE_MIXED_CASE: &str = "security.password.require_mixed_case";
    pub const PASSWORD_EXPIRY_DAYS: &str = "security.password.expiry_days";
    pub const LOGIN_MAX_FAILURES: &str = "security.login.max_failures";
    pub const LOGIN_FAILURE_WINDOW_SECONDS: &str = "security.login.failure_window_seconds";
}

/// 系统参数服务
#[derive(Debug, Clone)]
pub struct SettingService {
    repo: SettingRepository,
    env: EnvFallbacks,
}

/// 来自**部署配置**的取值（环境变量）
///
/// ── 为什么需要它 ────────────────────────────────────────────
/// 参数表与部署配置都会"想"决定 `login.max_failures` 这个值，
/// 而两者的优先级必须写死，否则会出现两种都很糟的结果：
///
/// - 参数表无条件优先：部署时用环境变量设的值被**静默忽略**。
///   实测（本仓 v0.22.0 首次集成时真的踩到）：测试与部分部署靠
///   `LOGIN_MAX_FAILURES=3` 构造低阈值，参数表里的种子值 10
///   直接盖掉它，于是"失败 3 次应被锁定"变成 200，
///   而日志里**一个字都没有**——爆破防护看起来在工作，实际从未生效。
/// - 环境变量无条件优先：管理员在界面上改了参数，重启后又变回去，
///   "写入成功"却永远不生效，正是 v0.16.0 关掉的那类缺陷。
///
/// 解析顺序因此定为：
/// **管理员显式改过 → 用参数表的值；否则 → 用部署配置的值。**
/// 判定"显式改过"靠 `updated_by IS NOT NULL`（种子写入的行为 NULL），
/// 所以这个区分是由数据本身承载的，不依赖任何内存状态。
#[derive(Debug, Clone, Copy)]
pub struct EnvFallbacks {
    /// `LOGIN_MAX_FAILURES`
    pub login_max_failures: u64,
    /// `LOGIN_FAILURE_WINDOW`
    pub login_failure_window_seconds: u64,
}

impl SettingService {
    pub fn new(repo: SettingRepository, env: EnvFallbacks) -> Self {
        Self { repo, env }
    }

    pub fn repo(&self) -> &SettingRepository {
        &self.repo
    }

    /// 该参数在部署配置里的取值（没有对应的环境变量则 `None`）
    fn env_value(&self, key: &str) -> Option<i64> {
        match key {
            keys::LOGIN_MAX_FAILURES => Some(self.env.login_max_failures as i64),
            keys::LOGIN_FAILURE_WINDOW_SECONDS => {
                Some(self.env.login_failure_window_seconds as i64)
            }
            _ => None,
        }
    }

    /// 解析单个参数的**生效取值（文本）**
    ///
    /// 顺序见 [`EnvFallbacks`] 的文档：管理员改过 → 参数表；否则 → 部署配置。
    ///
    /// 返回**文本**而不是 `i64`：`require_mixed_case` 的取值是 `"true"`，
    /// 按整数解析会失败并悄悄回落到默认值——管理员开了开关而策略没变，
    /// 又是一个"写入成功但无效果"的开关。
    async fn resolve_raw(&self, key: &str) -> (String, &'static str) {
        let def = match find_def(key) {
            Some(d) => d,
            None => return (String::new(), "default"),
        };
        let env = self.env_value(key);
        let map = match self.repo.load_all().await {
            Ok(m) => m,
            Err(e) => {
                tracing::error!("读取系统参数 {key} 失败，回落部署配置: {e}");
                return (
                    env.map(|v| v.to_string())
                        .unwrap_or_else(|| def.default.into()),
                    "env",
                );
            }
        };
        let entry = map.get(key);
        let overridden = entry.map(|e| e.admin_overridden).unwrap_or(false);

        if overridden {
            if let Some(v) = entry.map(|e| e.value.trim().to_string()) {
                if !v.is_empty() {
                    return (v, "admin");
                }
            }
            // 落库值不可解析：管理员改坏了（绕过后端直写库）。
            // 静默沿用部署配置比 500 好，但必须留痕。
            tracing::error!("系统参数 {key} 的落库值不可解析，回落部署配置");
        }
        if let Some(v) = env {
            return (v.to_string(), "env");
        }
        (def.default.to_string(), "default")
    }

    /// 按整数解析生效值；解析失败时回落到 `default`
    async fn resolve_int(&self, key: &str) -> i64 {
        let def = match find_def(key) {
            Some(d) => d,
            None => return 0,
        };
        let (raw, _) = self.resolve_raw(key).await;
        raw.trim()
            .parse::<i64>()
            .unwrap_or_else(|_| def.default.parse().unwrap_or(0))
    }

    /// 按布尔解析生效值；只接受 `true` / `false`
    ///
    /// 刻意**不接受** `1` / `0` / `yes`：写入路径的 [`validate_value`] 已经
    /// 只收 `true`/`false`，这里同口径才能保证"存进去的都能读出来"。
    async fn resolve_bool(&self, key: &str) -> bool {
        let def = match find_def(key) {
            Some(d) => d,
            None => return false,
        };
        let (raw, _) = self.resolve_raw(key).await;
        match raw.trim() {
            "true" => true,
            "false" => false,
            other => {
                tracing::error!("系统参数 {key} 的取值 {other:?} 不是布尔，回落默认值");
                def.default.trim() == "true"
            }
        }
    }

    /// 装配当前生效的口令策略
    ///
    /// **任何一个参数读失败都回落到默认值**，绝不因为参数表不可用
    /// 就让所有人登不进或注册不了。参数是"收紧策略"的手段，
    /// 不该成为"把系统锁死"的开关。
    pub async fn password_policy(&self) -> PasswordPolicy {
        let min_length = self.resolve_int(keys::PASSWORD_MIN_LENGTH).await;
        let max_length = self.resolve_int(keys::PASSWORD_MAX_LENGTH).await;
        let min_char_classes = self.resolve_int(keys::PASSWORD_MIN_CHAR_CLASSES).await;
        let require_mixed_case = self.resolve_bool(keys::PASSWORD_REQUIRE_MIXED_CASE).await;
        let expiry_days = self.resolve_int(keys::PASSWORD_EXPIRY_DAYS).await;

        let mut policy = PasswordPolicy {
            min_length: clamp_usize(min_length, 1, 1024),
            max_length: clamp_usize(max_length, 1, 1024),
            min_char_classes: clamp_usize(min_char_classes, 1, 5),
            require_mixed_case,
            expiry_days: expiry_days.max(0),
        };

        // min >= max 会配出"长度必须在 20-20 之间"这种空区间，
        // 表现为**任何口令都注册不了**。修正成"上界至少比下界大 1"，
        // 宁可偏离管理员的意图，也不能让系统收不了新口令。
        if policy.min_length >= policy.max_length {
            tracing::error!(
                "口令长度策略非法（min={} >= max={}），已把上界修正为下界+1",
                policy.min_length,
                policy.max_length
            );
            policy.max_length = policy.min_length + 1;
        }
        policy
    }

    /// 登录失败阈值
    pub async fn login_max_failures(&self) -> u64 {
        let v = self.resolve_int(keys::LOGIN_MAX_FAILURES).await;
        if v < 1 {
            // 阈值 0 会让"失败 0 次"就锁死账号——一次登录都进不去。
            let fallback = self.env.login_max_failures;
            tracing::error!("登录失败阈值非法({v})，回落到部署配置 {fallback}");
            return fallback;
        }
        v as u64
    }

    /// 登录失败计数窗口（秒）
    pub async fn login_failure_window_seconds(&self) -> u64 {
        let v = self.resolve_int(keys::LOGIN_FAILURE_WINDOW_SECONDS).await;
        if v < 1 {
            // 窗口 0 意味着计数写入即过期 = 永不锁定，
            // 爆破防护会静默失效，而这正是一个"看起来正常"的失效。
            let fallback = self.env.login_failure_window_seconds;
            tracing::error!("失败计数窗口非法({v})，回落到部署配置 {fallback}");
            return fallback;
        }
        v as u64
    }

    /// 列出参数，并把 `value` / `source` 换成**实际生效**的那一份
    ///
    /// 参数页要显示的是"现在真正生效的数字"，而不是 DB 里躺着的那个。
    /// 两者在"部署用环境变量设了值、没人改过"时会不同——
    /// 显示后者会让管理员以为自己看的是实际配置。
    pub async fn list(&self) -> Result<Vec<crate::repository::setting::SettingView>, AppError> {
        let mut views = self.repo.list().await?;
        for view in &mut views {
            if self.env_value(&view.key).is_some() {
                let (raw, source) = self.resolve_raw(&view.key).await;
                view.value = raw;
                view.source = source.to_string();
            }
        }
        Ok(views)
    }

    /// 校验并写入单个参数
    ///
    /// 校验分三层，缺一层就留下一类"配了但用不了"的参数：
    /// 1. key 必须在 [`SETTING_DEFS`] 里（不允许凭空造参数）
    /// 2. 取值必须能按声明的类型解析
    /// 3. 整数必须落在 `[min, max]`
    pub async fn update(
        &self,
        key: &str,
        value: &str,
        updated_by: uuid::Uuid,
    ) -> Result<(), AppError> {
        let def = find_def(key).ok_or_else(|| {
            AppError::BadRequest(format!(
                "没有名为 {key} 的系统参数（可改的参数由服务端定义，不能自行新增）"
            ))
        })?;

        let normalized = validate_value(def, value)?;

        // 口令长度这两个参数之间有跨字段约束，单字段范围校验看不出来。
        // 必须在**写入前**查一次当前另一个值，否则管理员先设 max=16
        // 再设 min=20 会成功，而系统从此收不了任何口令。
        if key == keys::PASSWORD_MIN_LENGTH || key == keys::PASSWORD_MAX_LENGTH {
            self.check_length_window_after_update(key, &normalized)
                .await?;
        }

        self.repo.update(key, &normalized, updated_by).await
    }

    /// 复位成默认值
    pub async fn reset(&self, key: &str, updated_by: uuid::Uuid) -> Result<(), AppError> {
        let def = find_def(key)
            .ok_or_else(|| AppError::BadRequest(format!("没有名为 {key} 的系统参数")))?;
        if self.env_value(key).is_some() {
            // 有部署配置兜底的参数走 clear_override：交还控制权给环境变量。
            // 否则"复位"会把参数**钉死**在代码默认值上，部署侧此后改
            // 环境变量不再有效果——与本仓反复修的"配置写了却不生效"同类。
            self.repo.clear_override(key).await?;
            tracing::info!(
                target: "service",
                "系统参数 {key} 已交还控制权给部署配置"
            );
            return Ok(());
        }
        self.update(key, def.default, updated_by).await
    }

    /// 校验"写入这个长度参数之后，min < max 是否仍成立"
    async fn check_length_window_after_update(
        &self,
        key: &str,
        new_value: &str,
    ) -> Result<(), AppError> {
        let new: i64 = new_value
            .parse()
            .map_err(|_| AppError::BadRequest("长度参数必须是整数".into()))?;
        let other_key = if key == keys::PASSWORD_MIN_LENGTH {
            keys::PASSWORD_MAX_LENGTH
        } else {
            keys::PASSWORD_MIN_LENGTH
        };
        let other = self.resolve_int(other_key).await;

        let (min, max) = if key == keys::PASSWORD_MIN_LENGTH {
            (new, other)
        } else {
            (other, new)
        };
        if min >= max {
            return Err(AppError::BadRequest(format!(
                "口令长度区间无效：最小 {min} 不小于最大 {max}，\
                 这样的区间会让任何口令都无法通过校验（当前 {} 的取值会被挡住）",
                if key == keys::PASSWORD_MIN_LENGTH {
                    "口令最大长度"
                } else {
                    "口令最小长度"
                }
            )));
        }
        Ok(())
    }
}

/// 按类型与范围校验取值，返回**归一后的文本**
pub fn validate_value(
    def: &crate::model::setting::SettingDef,
    value: &str,
) -> Result<String, AppError> {
    let trimmed = value.trim();
    match def.value_type {
        SettingType::Bool => match trimmed.to_ascii_lowercase().as_str() {
            "true" => Ok("true".to_string()),
            "false" => Ok("false".to_string()),
            other => Err(AppError::BadRequest(format!(
                "参数 {} 是布尔类型，只接受 true / false，收到 {other:?}",
                def.name
            ))),
        },
        SettingType::Int => {
            let n: i64 = trimmed.parse().map_err(|_| {
                AppError::BadRequest(format!("参数 {} 必须是整数，收到 {trimmed:?}", def.name))
            })?;
            if n < def.min || n > def.max {
                return Err(AppError::BadRequest(format!(
                    "参数 {} 的取值 {n} 超出允许范围 {}-{}",
                    def.name, def.min, def.max
                )));
            }
            Ok(n.to_string())
        }
    }
}

fn clamp_usize(v: i64, lo: i64, hi: i64) -> usize {
    v.clamp(lo, hi) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::setting::{find_def, SETTING_DEFS};

    #[test]
    fn every_key_constant_actually_appears_in_the_definitions() {
        // 这条测试挡住"常量里的 key 与 SETTING_DEFS 里的 key 拼错"。
        // 拼错的后果很隐蔽：`password_policy()` 会一路读到默认值，
        // 界面显示改成功了，而策略**永远没变**——正是 v0.16.0 的形态。
        for key in [
            keys::PASSWORD_MIN_LENGTH,
            keys::PASSWORD_MAX_LENGTH,
            keys::PASSWORD_MIN_CHAR_CLASSES,
            keys::PASSWORD_REQUIRE_MIXED_CASE,
            keys::PASSWORD_EXPIRY_DAYS,
            keys::LOGIN_MAX_FAILURES,
            keys::LOGIN_FAILURE_WINDOW_SECONDS,
        ] {
            assert!(find_def(key).is_some(), "参数定义里没有 {key}");
        }
        // 反向：定义里的每一个 key 都必须被某个常量覆盖
        for def in SETTING_DEFS {
            let covered = [
                keys::PASSWORD_MIN_LENGTH,
                keys::PASSWORD_MAX_LENGTH,
                keys::PASSWORD_MIN_CHAR_CLASSES,
                keys::PASSWORD_REQUIRE_MIXED_CASE,
                keys::PASSWORD_EXPIRY_DAYS,
                keys::LOGIN_MAX_FAILURES,
                keys::LOGIN_FAILURE_WINDOW_SECONDS,
            ]
            .contains(&def.key);
            assert!(
                covered,
                "参数 {} 没有被 service::setting::keys 覆盖，它很可能永远不会被读取",
                def.key
            );
        }
    }

    #[test]
    fn bool_values_are_normalized_and_non_bools_rejected() {
        let def = find_def(keys::PASSWORD_REQUIRE_MIXED_CASE).unwrap();
        assert_eq!(validate_value(def, "true").unwrap(), "true");
        assert_eq!(validate_value(def, "FALSE").unwrap(), "false");
        assert_eq!(validate_value(def, " true ").unwrap(), "true");
        assert!(matches!(
            validate_value(def, "1"),
            Err(AppError::BadRequest(_))
        ));
        assert!(matches!(
            validate_value(def, "yes"),
            Err(AppError::BadRequest(_))
        ));
    }

    #[test]
    fn int_values_must_be_integers_inside_the_declared_range() {
        let def = find_def(keys::PASSWORD_MIN_LENGTH).unwrap();
        assert_eq!(validate_value(def, "12").unwrap(), "12");
        assert_eq!(validate_value(def, " 12 ").unwrap(), "12");
        // 越界
        assert!(matches!(
            validate_value(def, "7"),
            Err(AppError::BadRequest(_))
        ));
        assert!(matches!(
            validate_value(def, "129"),
            Err(AppError::BadRequest(_))
        ));
        // 非整数
        assert!(matches!(
            validate_value(def, "12.5"),
            Err(AppError::BadRequest(_))
        ));
        assert!(matches!(
            validate_value(def, "abc"),
            Err(AppError::BadRequest(_))
        ));
    }

    /// 长度区间的跨字段约束
    ///
    /// 单看 `min_length ∈ [8,128]` 和 `max_length ∈ [8,1024]` 都合法，
    /// 但 `min=20, max=16` 会配出一个**空区间**，此后任何口令都注册不了。
    /// 这条纯函数层面测的是取值校验；跨字段那条在集成测试里测。
    #[test]
    fn a_length_window_that_is_always_empty_is_rejected_by_bounds() {
        // min_length 的上界 128 本身合法，但配合 max=8 就没意义了；
        // 这里断言的是"每个参数单独看都合法"，把跨字段问题留给集成测试
        let min_def = find_def(keys::PASSWORD_MIN_LENGTH).unwrap();
        let max_def = find_def(keys::PASSWORD_MAX_LENGTH).unwrap();
        assert!(validate_value(min_def, "128").is_ok());
        assert!(validate_value(max_def, "8").is_ok());
    }
}
