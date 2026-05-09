-- 字典类型表
CREATE TABLE IF NOT EXISTS dict_types (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    code        VARCHAR(100) NOT NULL UNIQUE,   -- 字典编码（如 gender, status）
    name        VARCHAR(100) NOT NULL,           -- 字典名称（如 性别，状态）
    description TEXT,
    status      VARCHAR(10) NOT NULL DEFAULT 'enabled' CHECK (status IN ('enabled', 'disabled')),
    sort_order  INTEGER NOT NULL DEFAULT 0,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- 字典项表
CREATE TABLE IF NOT EXISTS dict_items (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    dict_type_id  UUID NOT NULL REFERENCES dict_types(id) ON DELETE CASCADE,
    label         VARCHAR(100) NOT NULL,         -- 展示标签（如 男, 女）
    value         VARCHAR(100) NOT NULL,         -- 实际值（如 1, 2）
    sort_order    INTEGER NOT NULL DEFAULT 0,
    status        VARCHAR(10) NOT NULL DEFAULT 'enabled' CHECK (status IN ('enabled', 'disabled')),
    is_default    BOOLEAN NOT NULL DEFAULT FALSE,
    color         VARCHAR(50),                   -- 标签颜色
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_dict_types_code ON dict_types(code);
CREATE INDEX IF NOT EXISTS idx_dict_items_type ON dict_items(dict_type_id);
