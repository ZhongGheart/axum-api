//! 系统参数定义（单一数据源）
//!
//! 与 [`crate::model::permission`] 同构的设计：参数**由代码定义**，
//! DB 只负责持久化取值。管理员能改的是"已定义参数的取值"，
//! 不能凭空造一个新参数出来——否则就多了一套没有校验、没有边界的
//! 自由格式键值对，任何登录用户都能往里塞东西。
//!
//! - [`SETTING_DEFS`] 定义每个参数的 key、类型、默认值与取值范围
//! - 迁移 `016` 用这里的默认值写入种子行
//! - 服务层按 `key` 类型化读取，解析失败回落到 `default`
//! - 前端「系统参数」页由 `GET /api/admin/settings` 驱动，**不硬编码参数清单**
//!
//! ── 为什么每个参数都要 `consumed_by` ────────────────────────
//! 这个仓反复出现过的缺陷形态是"配置存在但无效果"：字典的三个开关
//! 写入都返回 200、界面都正常，但对实际行为毫无影响（v0.16.0）。
//! 所以每个参数都必须写明**它被哪段代码读**，并在参数管理页显示出来。
//! 一个说不出消费方的参数不该被加进来。

use serde::{Deserialize, Serialize};

/// 参数取值类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum SettingType {
    /// 布尔
    Bool,
    /// 整数
    Int,
}

impl SettingType {
    /// 前端渲染用的类型名（供参数管理页选择对应控件）
    pub fn as_str(&self) -> &'static str {
        match self {
            SettingType::Bool => "bool",
            SettingType::Int => "int",
        }
    }
}

/// 参数分组（前端按此分节）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum SettingGroup {
    /// 口令策略
    Password,
    /// 登录防护
    Login,
    /// 注册准入
    Registration,
    /// 会话
    Session,
}

impl SettingGroup {
    /// 前端分组标识
    pub fn as_str(&self) -> &'static str {
        match self {
            SettingGroup::Password => "password",
            SettingGroup::Login => "login",
            SettingGroup::Registration => "registration",
            SettingGroup::Session => "session",
        }
    }
}

/// 单个参数的定义
#[derive(Debug, Clone, Copy)]
pub struct SettingDef {
    /// 参数名（DB 主键，必须与迁移里的种子逐字一致）
    pub key: &'static str,
    /// 界面上显示的名称
    pub name: &'static str,
    /// 用途说明。这段话会直接出现在管理页上，
    /// 所以要写"改了会发生什么"，而不是复述参数名
    pub description: &'static str,
    /// 取值类型
    pub value_type: SettingType,
    /// 分组
    pub group: SettingGroup,
    /// 默认值（以文本表示，与 DB 存储形式一致）
    pub default: &'static str,
    /// 整数下界；`Bool` 参数忽略此字段
    pub min: i64,
    /// 整数上界；`Bool` 参数忽略此字段
    pub max: i64,
    /// 哪些代码读它（管理页展示用，防止再出现"无效果的开关"）
    pub consumed_by: &'static str,
}

/// 全部参数定义
///
/// **默认值必须与本版此前的硬编码常量逐字一致**：
/// [`crate::utils::validation::PASSWORD_MIN_LEN`] = 8、
/// [`crate::utils::validation::PASSWORD_MAX_LEN`] = 128、
/// 至少 2 类字符、不要求大小写混合、登录失败阈值 10 / 窗口 300 秒。
/// 由 `settings_defaults_match_the_hardcoded_constants` 钉住。
pub const SETTING_DEFS: &[SettingDef] = &[
    SettingDef {
        key: "security.password.min_length",
        name: "口令最小长度",
        description: "新口令的最少字符数。抬高它**不会**要求存量用户立刻改口令——复杂度只在设置口令时校验，不在登录时校验。",
        value_type: SettingType::Int,
        group: SettingGroup::Password,
        default: "8",
        min: 8,
        max: 128,
        consumed_by: "设置/修改口令时校验（不作用于登录校验）",
    },
    SettingDef {
        key: "security.password.max_length",
        name: "口令最大长度",
        description: "新口令的最多字符数，按**字符**计不是字节。上限过大没有收益，下限必须小于「口令最小长度」。",
        value_type: SettingType::Int,
        group: SettingGroup::Password,
        default: "128",
        min: 8,
        max: 1024,
        consumed_by: "设置/修改口令时校验（不作用于登录校验）",
    },
    SettingDef {
        key: "security.password.min_char_classes",
        name: "最少字符类别数",
        description: "口令至少要包含几类字符。五类分别是：ASCII 大写、ASCII 小写、数字、符号、非 ASCII 字母。设为 1 等于不校验复杂度。",
        value_type: SettingType::Int,
        group: SettingGroup::Password,
        default: "2",
        min: 1,
        max: 5,
        consumed_by: "设置/修改口令时校验（不作用于登录校验）",
    },
    SettingDef {
        key: "security.password.require_mixed_case",
        name: "强制大小写混合",
        description: "开启后口令必须同时含 ASCII 大写与 ASCII 小写。注意这会误伤纯中文口令——汉字不含 ASCII 大小写。",
        value_type: SettingType::Bool,
        group: SettingGroup::Password,
        default: "false",
        min: 0,
        max: 1,
        consumed_by: "设置/修改口令时校验（不作用于登录校验）",
    },
    SettingDef {
        key: "security.password.expiry_days",
        name: "口令有效期（天）",
        description: "口令设置后多少天必须更换。**0 表示永不过期**（默认）。开启后存量口令按其设置时刻起算，多数会立即被要求改密——这正是启用该策略的目的。",
        value_type: SettingType::Int,
        group: SettingGroup::Password,
        default: "0",
        min: 0,
        max: 3650,
        consumed_by: "登录时判定是否下发受限令牌（不阻断登录本身）",
    },
    SettingDef {
        key: "security.login.max_failures",
        name: "登录失败锁定阈值",
        description: "同一账号或同一 IP 在窗口内累计失败达到该次数后锁定。调低会更容易误伤记错口令的正常用户。",
        value_type: SettingType::Int,
        group: SettingGroup::Login,
        default: "10",
        min: 1,
        max: 1000,
        consumed_by: "登录前的锁定判定（账号与 IP 两个维度各自独立计数）",
    },
    SettingDef {
        key: "security.login.failure_window_seconds",
        name: "失败计数窗口（秒）",
        description: "失败计数在 Redis 里的存活时间。窗口过后失败计数自动清零，因此一次慢速爆破只要每次间隔够久就不会被锁。",
        value_type: SettingType::Int,
        group: SettingGroup::Login,
        default: "300",
        min: 1,
        max: 86400,
        consumed_by: "写入失败计数时的 Redis TTL",
    },
    SettingDef {
        key: "security.registration.enabled",
        name: "开放注册",
        description: "关闭后 `/api/auth/register` 一律返回 403，**已注册用户不受影响**。公网部署若不需要自助注册，应关闭它——否则任何人都能注册并自动获得 `user` 角色。",
        value_type: SettingType::Bool,
        group: SettingGroup::Registration,
        default: "true",
        min: 0,
        max: 1,
        consumed_by: "注册入口的准入判定（`service::auth::register` 的第一道检查）",
    },
    SettingDef {
        key: "security.session.max_concurrent",
        name: "并发会话上限",
        description: "同一账号最多同时在线几个会话。**0 表示不限制**（默认）。达到上限后新登录会被拒绝，用户需先下线其他设备（个人中心 → 登录会话）。",
        value_type: SettingType::Int,
        group: SettingGroup::Session,
        default: "0",
        min: 0,
        max: 100,
        consumed_by: "登录时判定是否还有空位（口令校验之后、登记新会话之前）",
    },
];

/// 按 key 查定义
pub fn find_def(key: &str) -> Option<&'static SettingDef> {
    SETTING_DEFS.iter().find(|d| d.key == key)
}

/// 口令策略（由参数表装配出的类型化视图）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasswordPolicy {
    /// 最小长度（字符）
    pub min_length: usize,
    /// 最大长度（字符）
    pub max_length: usize,
    /// 最少字符类别数
    pub min_char_classes: usize,
    /// 是否强制大小写混合
    pub require_mixed_case: bool,
    /// 口令有效期（天）；0 表示不过期
    pub expiry_days: i64,
}

impl Default for PasswordPolicy {
    fn default() -> Self {
        Self {
            min_length: crate::utils::validation::PASSWORD_MIN_LEN,
            max_length: crate::utils::validation::PASSWORD_MAX_LEN,
            min_char_classes: crate::utils::validation::PASSWORD_MIN_CHAR_CLASSES,
            require_mixed_case: false,
            expiry_days: 0,
        }
    }
}

impl PasswordPolicy {
    /// 该策略下口令是否已过期
    ///
    /// `changed_at` 为 `None`（无法判断设置时刻）时**判为未过期**：
    /// 把"不知道"当成"已过期"，会在一次数据补齐之前就让全部用户登不进。
    pub fn is_expired(&self, changed_at: Option<chrono::DateTime<chrono::Utc>>) -> bool {
        if self.expiry_days <= 0 {
            return false;
        }
        let Some(changed_at) = changed_at else {
            return false;
        };
        let elapsed_days = (chrono::Utc::now() - changed_at).num_days();
        elapsed_days >= self.expiry_days
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn setting_keys_are_unique() {
        let mut seen = HashSet::new();
        for def in SETTING_DEFS {
            assert!(seen.insert(def.key), "参数名重复: {}", def.key);
        }
        assert_eq!(seen.len(), SETTING_DEFS.len());
    }

    /// 开放注册开关必须存在，且默认值必须是 `true`
    ///
    /// 两个方向都会出事：
    /// - 参数不存在 → `resolve_bool` 回落到 `find_def` 失败后的 `false`，
    ///   注册入口被静默关掉，而管理员在界面上看不到任何可以打开它的开关。
    /// - 默认值是 `false` → 一次常规发版突然关掉所有存量部署的注册入口，
    ///   包括那些**故意**开放注册的内部系统。
    #[test]
    fn registration_switch_exists_and_defaults_to_open() {
        let def =
            find_def("security.registration.enabled").expect("开放注册开关必须是已定义的系统参数");
        assert_eq!(def.value_type, SettingType::Bool);
        assert_eq!(
            def.default, "true",
            "默认值必须是 true：收紧要由管理员显式表达"
        );
        assert_eq!(def.group, SettingGroup::Registration);
    }

    /// 并发会话上限必须存在，且默认值必须是 `0`（不限制）
    ///
    /// 默认非 0 会让一次常规发版突然只允许有限设备登录——
    /// 与注册开关同一原则：默认值必须与本版此前的行为逐字一致。
    #[test]
    fn concurrent_session_limit_exists_and_defaults_to_unlimited() {
        let def = find_def("security.session.max_concurrent")
            .expect("并发会话上限必须是已定义的系统参数");
        assert_eq!(def.value_type, SettingType::Int);
        assert_eq!(def.default, "0", "默认值必须是 0（不限制）");
        assert_eq!(def.group, SettingGroup::Session);
    }

    /// 默认值必须与本版此前的硬编码常量一致
    ///
    /// 这条是本仓最贵的一课：v0.11.0 抬口令门槛时默认值取错，
    /// 等于对全部存量用户叠加一次强制改密。参数表的默认值同样
    /// 是"改代码那一刻就生效"的东西，一旦与旧常量漂移，
    /// 光是部署一个新版本就能把所有用户锁在门外。
    #[test]
    fn settings_defaults_match_the_hardcoded_constants() {
        let d = PasswordPolicy::default();
        assert_eq!(d.min_length, crate::utils::validation::PASSWORD_MIN_LEN);
        assert_eq!(d.max_length, crate::utils::validation::PASSWORD_MAX_LEN);
        assert_eq!(
            d.min_char_classes,
            crate::utils::validation::PASSWORD_MIN_CHAR_CLASSES
        );
        // 过期策略此前不存在，默认为"不过期"
        assert_eq!(d.expiry_days, 0);
        assert!(!d.require_mixed_case);
    }

    /// 每个参数都必须能说出自己被谁消费
    ///
    /// v0.16.0 的整版主题就是"开关存在但无效果"。这条测试是那个教训的
    /// 机械化：一个说不清消费方的参数，本就不该存在于这张表里。
    #[test]
    fn every_setting_documents_its_consumer() {
        for def in SETTING_DEFS {
            assert!(
                def.consumed_by.len() > 8,
                "参数 {} 没有写明被哪段代码消费（这正是 v0.16.0 的缺陷形态）",
                def.key
            );
            assert!(!def.description.trim().is_empty(), "{} 缺少说明", def.key);
            assert!(!def.name.trim().is_empty(), "{} 缺少显示名", def.key);
        }
    }

    /// 取值范围必须自洽，否则管理员能配出一个永远无法满足的口令策略
    #[test]
    fn integer_bounds_are_sane() {
        for def in SETTING_DEFS {
            if def.value_type != SettingType::Int {
                continue;
            }
            assert!(
                def.min <= def.max,
                "{} 的范围反了: min={} > max={}",
                def.key,
                def.min,
                def.max
            );
            let default: i64 = def.default.parse().expect("默认值必须是整数");
            assert!(
                (def.min..=def.max).contains(&default),
                "{} 的默认值 {} 落在范围 [{}, {}] 之外",
                def.key,
                default,
                def.min,
                def.max
            );
        }
    }

    /// 默认策略下 min < max，否则会配出"口令长度必须在 8-8 之间"这种空区间
    #[test]
    fn default_policy_has_a_usable_length_window() {
        let d = PasswordPolicy::default();
        assert!(d.min_length < d.max_length);
    }

    /// 过期判定：0 天永不过期；未知的设置时刻判为未过期
    #[test]
    fn expiry_only_applies_when_a_window_is_configured() {
        let never = PasswordPolicy {
            expiry_days: 0,
            ..Default::default()
        };
        assert!(!never.is_expired(Some(chrono::Utc::now() - chrono::Duration::days(9999))));
        assert!(!never.is_expired(None));

        let ninety = PasswordPolicy {
            expiry_days: 90,
            ..Default::default()
        };
        assert!(ninety.is_expired(Some(chrono::Utc::now() - chrono::Duration::days(91))));
        assert!(!ninety.is_expired(Some(chrono::Utc::now() - chrono::Duration::days(89))));
        // "不知道设置时刻"不能等同于"已过期"：否则一次数据补齐
        // 之前所有用户都会被拦在改密页
        assert!(!ninety.is_expired(None));
    }
}
