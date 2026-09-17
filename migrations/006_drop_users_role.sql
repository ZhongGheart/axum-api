-- v0.2：角色唯一数据源 = user_roles 表
--
-- 此前 users.role 与 user_roles 并存：管理端改 users.role 不影响鉴权，
-- 改 user_roles 又不影响 JWT 中的主角色，两处会互相矛盾。
--
-- 升级路径：先把 users.role 回填进 user_roles（保留存量权限），再删除冗余列。
-- 全新库此时 roles 表尚为空，回填为空操作，不影响后续种子数据。
INSERT INTO user_roles (user_id, role_id)
SELECT u.id, r.id
FROM users u
JOIN roles r ON r.name = u.role
ON CONFLICT (user_id, role_id) DO NOTHING;

ALTER TABLE users DROP COLUMN IF EXISTS role;
