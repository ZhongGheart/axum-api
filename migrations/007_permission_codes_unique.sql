-- 权限码唯一性约束
--
-- 权限码是接口级授权的判定依据，必须唯一，否则
-- "查询用户拥有哪些权限码" 的结果会随 JOIN 顺序抖动，同一请求时通时不通。
--
-- 部分唯一索引（WHERE permission IS NOT NULL）：
-- 允许任意多条 permission IS NULL 的普通菜单行共存，只约束真实权限码。
-- 启动种子依赖这个索引做幂等 upsert（ON CONFLICT (permission)）。
-- 先检查重复：直接建唯一索引的话，Postgres 抛的是原始索引错误，
-- 管理员看不懂「permission」是什么。这里提前给出可操作的提示。
-- 刻意不自动去重：重复权限码说明菜单被手工改坏，应由管理员在「菜单管理」页修正，
-- 静默去重会掩盖问题。
DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM menus
        WHERE permission IS NOT NULL AND permission <> ''
        GROUP BY permission
        HAVING COUNT(*) > 1
    ) THEN
        RAISE EXCEPTION
            'menus.permission 存在重复权限码，请先在「菜单管理」页修正后再启动';
    END IF;
END
$$;

CREATE UNIQUE INDEX IF NOT EXISTS idx_menus_permission_unique
    ON menus (permission)
    WHERE permission IS NOT NULL AND permission <> '';
