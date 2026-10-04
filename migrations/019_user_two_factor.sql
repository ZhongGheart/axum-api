-- v0.25.0：两步验证（TOTP）
--
-- 背景：`totp|two_factor|2fa|otp` 在 src / migrations / frontend 三处 grep 均零命中。
-- 口令是唯一的登录凭据，泄露一次即等于泄露全部——本仓没有邮件通道，
-- 也就没有"改邮箱 / 重置口令"这条自助恢复路径，账号一旦被盗管理员只能
-- 手工改库。TOTP 是唯一能在**不引入外部服务凭据**的前提下加一层独立因子的做法。
--
-- ── 为什么密钥必须加密存，而不是明文 ──────────────────────
-- TOTP 密钥与口令的区别在于：口令是不可逆摘要（Argon2），数据库泄露拿不到原值；
-- 而 TOTP 密钥**必须可逆**才能算出验证码。数据库一旦泄露（备份、只读副本、
-- 运维误操作导出 SQL），明文密钥意味着攻击者可以为任意账号生成合法验证码，
-- 2FA 从此对已泄露的数据毫无意义。
-- 因此这里用 AES-256-GCM 加密后存，密钥由 `TOTP_ENCRYPTION_KEY` 环境变量提供，
-- 与数据库分离保管。
--
-- AES-GCM 是带认证的加密：密文被篡改会导致解密失败（返回 Err），
-- 而不是解出一段垃圾密钥后让所有验证码静默失效。
--
-- ── 为什么恢复码只存摘要 ─────────────────────────────────
-- 恢复码的定位是"设备丢了之后的一次性后门"。它与口令同为凭据，
-- 同样不应该可逆存储：SHA-256 摘要足够（8 位随机码熵远低于 Argon2 的适用区间，
-- 但它是一次性的、用掉即删，不存在离线爆破的现实窗口）。
-- 存成逗号分隔的摘要串，**不存明文**。

ALTER TABLE users
    ADD COLUMN IF NOT EXISTS totp_secret_enc  BYTEA,
    ADD COLUMN IF NOT EXISTS totp_enabled_at  TIMESTAMPTZ;

COMMENT ON COLUMN users.totp_secret_enc IS
    'AES-256-GCM 加密后的 TOTP 密钥（nonce||ciphertext||tag）。NULL 表示未绑定 2FA';
COMMENT ON COLUMN users.totp_enabled_at IS
    'TOTP 生效时间。NULL = 未启用；非 NULL = 登录时必须过第二道因子';

CREATE TABLE IF NOT EXISTS user_two_factor_recovery (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id     UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    code_hash   VARCHAR(64) NOT NULL,
    used_at     TIMESTAMPTZ,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_recovery_code_per_user UNIQUE (user_id, code_hash)
);

COMMENT ON TABLE user_two_factor_recovery IS
    '两步验证恢复码的 SHA-256 摘要。只存摘要不存明文；使用后写 used_at 而不是删除，'
    '这样"同一恢复码被用第二次"能被索引挡住而不是靠应用层记得查';

CREATE INDEX IF NOT EXISTS idx_two_factor_recovery_user
    ON user_two_factor_recovery (user_id);

-- 关闭 2FA 时连带清掉恢复码：留着它们等于留着一条绕过第二因子的后门。
-- 触发器而不是应用层 DELETE——应用层有 4 条可能遗漏其中一条的路径。
CREATE OR REPLACE FUNCTION cleanup_two_factor_on_disable() RETURNS TRIGGER AS $$
BEGIN
    IF OLD.totp_secret_enc IS NOT NULL AND NEW.totp_secret_enc IS NULL THEN
        DELETE FROM user_two_factor_recovery WHERE user_id = OLD.id;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_two_factor_cleanup ON users;
CREATE TRIGGER trg_two_factor_cleanup
    BEFORE UPDATE OF totp_secret_enc ON users
    FOR EACH ROW EXECUTE FUNCTION cleanup_two_factor_on_disable();

-- ── 刻意不做的三件事 ─────────────────────────────────────
-- 1. 不加 `totp_enabled BOOLEAN`：生效状态就是 `totp_enabled_at IS NOT NULL`，
--    多一列就多一个可能与实际不一致的状态。
-- 2. 不给 users 加 `failed_2fa_count`：二次验证的失败计数放 Redis（与登录
--    爆破计数同一套机制），DB 只存长期事实。
-- 3. 不在迁移里改任何存量行：本迁移纯加列加表，存量用户 `totp_enabled_at`
--    为 NULL 即"未启用 2FA"，登录流程与此前逐字一致。
