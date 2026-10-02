-- 权限码清空后的恢复凭据
--
-- `menus.permission` 被清空后，该码在全系统消失：没有任何角色再持有它，
-- 于是 `update_menu` 的"改写权限码必须持有目标码"守卫会把**写回**也一并
-- 拦死（没人持有 → 403）。这是一个死锁，管理员只能去新建一个孤儿按钮，
-- 原按钮的 role_menus 授权还在、却不再对应任何码。
--
-- 这里留一份"被清掉的码 + 谁清的"，让清空者可以撤销自己的误操作。
-- 安全性不依赖"能清空"这个动作本身（清空已授予角色的按钮要求持有该码，
-- 见 update_menu），而依赖"只有清空者能恢复"：恢复即回到清空前的状态，
-- 净零提权。
--
-- 刻意只存**一个**槽位而不是历史表：需要恢复的只有"上一次被清空的那个码"，
-- 多次清空/恢复循环里最后一个槽位总是对的；历史表会带来无人查阅的表
-- 与无界增长。
--
-- `cleared_by` 存 user id 而非用户名：用户名可改，id 不会。
-- 不加外键：菜单与其恢复凭据同生共死，且历史值指向的用户可能已被删除，
-- 加外键会让"删用户"被菜单凭据卡住。
ALTER TABLE menus
    ADD COLUMN IF NOT EXISTS prev_permission TEXT,
    ADD COLUMN IF NOT EXISTS prev_permission_cleared_by UUID;

COMMENT ON COLUMN menus.prev_permission IS
    '最近一次被清空掉的权限码，供 restore-permission 恢复用；当前有 permission 时无意义';
COMMENT ON COLUMN menus.prev_permission_cleared_by IS
    '清空 prev_permission 的操作者 id；只有本人可恢复，保证恢复是净零操作';
