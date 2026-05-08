-- 创建用户表
-- 使用 UUID 作为主键，支持分布式部署
-- 启用 uuid-ossp 扩展用于生成 UUID（如不支持可用纯 SQL 方式）

-- 创建 uuid-ossp 扩展（如果尚未创建）
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";

-- 创建用户表
CREATE TABLE IF NOT EXISTS users (
    -- 主键：UUID v4，自动生成
    id          UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    -- 用户名：唯一，3-50 个字符
    username    VARCHAR(50)  NOT NULL UNIQUE,
    -- 电子邮箱：唯一
    email       VARCHAR(255) NOT NULL UNIQUE,
    -- 密码哈希值：Argon2 格式
    password_hash TEXT       NOT NULL,
    -- 用户角色：admin 管理员 / user 普通用户
    role        VARCHAR(20)  NOT NULL DEFAULT 'user'
                            CHECK (role IN ('admin', 'user')),
    -- 是否激活
    is_active   BOOLEAN      NOT NULL DEFAULT TRUE,
    -- 创建时间（自动设置）
    created_at  TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    -- 更新时间（自动更新）
    updated_at  TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);

-- 创建更新时间自动更新的触发器函数
CREATE OR REPLACE FUNCTION update_updated_at_column()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- 为用户表添加自动更新触发器
CREATE TRIGGER set_users_updated_at
    BEFORE UPDATE ON users
    FOR EACH ROW
    EXECUTE FUNCTION update_updated_at_column();

-- 索引：加速用户名和邮箱的查询
CREATE INDEX idx_users_username ON users(username);
CREATE INDEX idx_users_email ON users(email);

-- 创建测试数据（可选，默认注释掉）
-- INSERT INTO users (username, email, password_hash, role)
-- VALUES ('admin', 'admin@example.com', '$argon2id$v=19$m=19456,t=2,p=1$...', 'admin');
