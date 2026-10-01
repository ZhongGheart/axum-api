-- v0.5.0：角色名归一化（trim + 小写）
--
-- 角色名是 RBAC 的授权键，不是展示文本：权限码按角色匹配、用户表单按名字提交。
-- v0.5.0 PR-2 起角色写入路径统一归一化，因此表里必须存 canonical 形式，
-- 否则会出现「下拉里选得到、提交后端归一化后查不到」的静默错配。
--
-- 存量库里可能已有 PR-1 的角色管理页建出来的非 canonical 名字（如 'Auditor'）。
--
-- 只处理大小写与首尾空白这两种**静默**错配；角色名中间有空格（如 'senior auditor'）
-- 是合法的，空格能原样往返，不在归一化范围内——否则迁移会把存量角色改成
-- 管理员没要求过的名字。
--
-- 刻意跳过归一化后与既有行冲突的名字，而不是让迁移失败：
-- 这属于需要管理员判断的数据问题（两个角色本来可能就是同一个角色），
-- 迁移期不该把整个应用卡在起不来。这些行保持原样，
-- 之后再用该名字建角色会拿到 409，由管理员显式处理。
--
-- user_roles / role_menus 一律按 role_id 外键关联，改名不影响已建立的授权关系。
--
-- 用 regexp_replace 裁两端空白（含 tab/换行）：PR-1 的建角色接口零校验，
-- '  \t ' 这种名字是能存进去的，只裁普通空格会留下 '\t' 这种仍然非法的名字。
-- **不要写成 btrim(name, '[:space:]')**：btrim 的第二参数是字符集合而非字符类，
-- 它会逐字符匹配 '[' ':' 's' 'p' 'a' 'c' 'e' ']'，
-- 实测会把 admin 裁成 dmin、把 auditor 裁成 uditor。
-- 全空白名字直接跳过——归一化后会变成空串，而空角色名是永远分配不了的死数据，
-- 不如原样留着让管理员看得见。
UPDATE roles r
SET name = lower(regexp_replace(r.name, '^[[:space:]]+|[[:space:]]+$', '', 'g'))
WHERE regexp_replace(r.name, '^[[:space:]]+|[[:space:]]+$', '', 'g') <> ''
  AND r.name <> lower(regexp_replace(r.name, '^[[:space:]]+|[[:space:]]+$', '', 'g'))
  AND NOT EXISTS (
      SELECT 1 FROM roles other
      WHERE other.name = lower(regexp_replace(r.name, '^[[:space:]]+|[[:space:]]+$', '', 'g'))
        AND other.id <> r.id
  );
