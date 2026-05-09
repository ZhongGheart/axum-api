-- 菜单表（树形结构）
CREATE TABLE IF NOT EXISTS menus (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    parent_id   UUID REFERENCES menus(id) ON DELETE CASCADE,
    name        VARCHAR(50)  NOT NULL,
    path        VARCHAR(200),
    component   VARCHAR(200),
    icon        VARCHAR(50),
    sort_order  INTEGER      NOT NULL DEFAULT 0,
    type        VARCHAR(20)  NOT NULL DEFAULT 'menu'
                            CHECK (type IN ('menu', 'button', 'directory')),
    permission  VARCHAR(100),
    is_visible  BOOLEAN      NOT NULL DEFAULT TRUE,
    created_at  TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

-- 角色-菜单关联表
CREATE TABLE IF NOT EXISTS role_menus (
    role_id     UUID NOT NULL REFERENCES roles(id) ON DELETE CASCADE,
    menu_id     UUID NOT NULL REFERENCES menus(id) ON DELETE CASCADE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (role_id, menu_id)
);

CREATE INDEX IF NOT EXISTS idx_role_menus_role_id ON role_menus(role_id);
CREATE INDEX IF NOT EXISTS idx_role_menus_menu_id ON role_menus(menu_id);
