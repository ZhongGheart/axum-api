//! OpenAPI 3.0 接口文档
//!
//! 手动构建 OpenAPI 规范 JSON，避免 utoipa 过程宏兼容性问题。
//! 端点：GET /api/openapi.json

use serde_json::json;

/// 返回完整的 OpenAPI 3.0 规范 JSON
pub fn openapi_json() -> serde_json::Value {
    json!({
        "openapi": "3.0.3",
        "info": {
            "title": "Axum Admin API",
            "version": "1.0.0",
            "description": "基于 Axum + SQLx + JWT 的生产级 Rust 后端 API"
        },
        "servers": [
            { "url": "/api", "description": "当前服务器" }
        ],
        "paths": {
            // ── 系统 ──
            "/api/health": {
                "get": {
                    "tags": ["系统"],
                    "summary": "健康检查",
                    "responses": {
                        "200": { "description": "服务正常", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ApiResponse_String" } } } }
                    }
                }
            },
            "/api/admin/test": {
                "get": {
                    "tags": ["系统"],
                    "summary": "管理员权限测试",
                    "security": [{ "bearer_auth": [] }],
                    "responses": { "200": { "description": "管理员测试接口" } }
                }
            },
            // ── 认证 ──
            "/api/auth/register": {
                "post": {
                    "tags": ["认证"],
                    "summary": "用户注册",
                    "requestBody": { "content": { "application/json": { "schema": { "$ref": "#/components/schemas/RegisterRequest" } } } },
                    "responses": {
                        "200": { "description": "注册成功", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ApiResponse_UserInfo" } } } },
                        "400": { "description": "参数错误" },
                        "409": { "description": "用户名或邮箱冲突" }
                    }
                }
            },
            "/api/auth/login": {
                "post": {
                    "tags": ["认证"],
                    "summary": "用户登录",
                    "requestBody": { "content": { "application/json": { "schema": { "$ref": "#/components/schemas/LoginRequest" } } } },
                    "responses": {
                        "200": { "description": "登录成功", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ApiResponse_LoginResponse" } } } },
                        "401": { "description": "用户名或密码错误" }
                    }
                }
            },
            "/api/auth/me": {
                "get": {
                    "tags": ["认证"],
                    "summary": "获取当前用户信息",
                    "security": [{ "bearer_auth": [] }],
                    "responses": {
                        "200": { "description": "用户信息", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ApiResponse_UserInfo" } } } }
                    }
                }
            },
            "/api/auth/logout": {
                "post": {
                    "tags": ["认证"],
                    "summary": "用户登出",
                    "security": [{ "bearer_auth": [] }],
                    "responses": { "200": { "description": "登出成功" } }
                }
            },
            // ── 用户管理 ──
            "/api/admin/users": {
                "get": {
                    "tags": ["用户管理"],
                    "summary": "用户列表（分页）",
                    "parameters": [
                        { "name": "page", "in": "query", "schema": { "type": "integer" }, "description": "页码" },
                        { "name": "page_size", "in": "query", "schema": { "type": "integer" }, "description": "每页条数" }
                    ],
                    "security": [{ "bearer_auth": [] }],
                    "responses": {
                        "200": { "description": "用户列表" },
                        "403": { "description": "权限不足" }
                    }
                },
                "post": {
                    "tags": ["用户管理"],
                    "summary": "创建用户",
                    "security": [{ "bearer_auth": [] }],
                    "requestBody": { "content": { "application/json": { "schema": { "$ref": "#/components/schemas/UserManageRequest" } } } },
                    "responses": {
                        "200": { "description": "创建成功" },
                        "400": { "description": "参数错误" }
                    }
                }
            },
            "/api/admin/users/{id}": {
                "put": {
                    "tags": ["用户管理"],
                    "summary": "更新用户",
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "security": [{ "bearer_auth": [] }],
                    "responses": { "200": { "description": "更新成功" } }
                },
                "delete": {
                    "tags": ["用户管理"],
                    "summary": "删除用户",
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "security": [{ "bearer_auth": [] }],
                    "responses": { "200": { "description": "删除成功" } }
                }
            },
            "/api/admin/users/batch-delete": {
                "post": {
                    "tags": ["用户管理"], "summary": "批量删除",
                    "security": [{ "bearer_auth": [] }],
                    "responses": { "200": { "description": "批量删除成功" } }
                }
            },
            "/api/admin/users/{id}/status": {
                "put": {
                    "tags": ["用户管理"], "summary": "切换状态",
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "security": [{ "bearer_auth": [] }],
                    "responses": { "200": { "description": "状态已更新" } }
                }
            },
            "/api/admin/users/{id}/reset-password": {
                "post": {
                    "tags": ["用户管理"], "summary": "重置密码",
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "security": [{ "bearer_auth": [] }],
                    "responses": { "200": { "description": "密码已重置" } }
                }
            },
            "/api/admin/users/{id}/roles": {
                "get": {
                    "tags": ["角色管理"], "summary": "用户角色列表",
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "security": [{ "bearer_auth": [] }],
                    "responses": { "200": { "description": "角色列表" } }
                },
                "post": {
                    "tags": ["角色管理"], "summary": "为用户分配角色",
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "security": [{ "bearer_auth": [] }],
                    "responses": { "200": { "description": "角色分配成功" } }
                },
                "put": {
                    "tags": ["用户管理"], "summary": "全量角色分配",
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "security": [{ "bearer_auth": [] }],
                    "responses": { "200": { "description": "角色分配成功" } }
                }
            },
            // ── 角色管理 ──
            "/api/admin/roles": {
                "get": {
                    "tags": ["角色管理"], "summary": "角色列表",
                    "security": [{ "bearer_auth": [] }],
                    "responses": { "200": { "description": "角色列表 (含用户数)" } }
                },
                "post": {
                    "tags": ["角色管理"], "summary": "新增角色",
                    "security": [{ "bearer_auth": [] }],
                    "responses": { "200": { "description": "角色创建成功" } }
                }
            },
            "/api/admin/roles/{id}": {
                "put": {
                    "tags": ["角色管理"], "summary": "更新角色",
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "security": [{ "bearer_auth": [] }],
                    "responses": { "200": { "description": "角色更新成功" } }
                },
                "delete": {
                    "tags": ["角色管理"], "summary": "删除角色",
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "security": [{ "bearer_auth": [] }],
                    "responses": { "200": { "description": "角色删除成功" } }
                }
            },
            "/api/admin/roles/{id}/menus": {
                "put": {
                    "tags": ["菜单管理"], "summary": "分配菜单权限",
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "security": [{ "bearer_auth": [] }],
                    "responses": { "200": { "description": "权限分配成功" } }
                }
            },
            // ── 菜单管理 ──
            "/api/admin/menus": {
                "get": {
                    "tags": ["菜单管理"], "summary": "获取菜单树",
                    "security": [{ "bearer_auth": [] }],
                    "responses": { "200": { "description": "菜单树" } }
                },
                "post": {
                    "tags": ["菜单管理"], "summary": "新增菜单",
                    "security": [{ "bearer_auth": [] }],
                    "responses": { "200": { "description": "创建成功" } }
                }
            },
            "/api/admin/menus/{id}": {
                "put": {
                    "tags": ["菜单管理"], "summary": "更新菜单",
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "security": [{ "bearer_auth": [] }],
                    "responses": { "200": { "description": "更新成功" } }
                },
                "delete": {
                    "tags": ["菜单管理"], "summary": "删除菜单",
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "security": [{ "bearer_auth": [] }],
                    "responses": { "200": { "description": "删除成功" } }
                }
            },
            // ── 数据字典 ──
            "/api/admin/dict/types": {
                "get": {
                    "tags": ["数据字典"], "summary": "字典类型列表",
                    "security": [{ "bearer_auth": [] }],
                    "responses": { "200": { "description": "字典类型列表" } }
                },
                "post": {
                    "tags": ["数据字典"], "summary": "新增字典类型",
                    "security": [{ "bearer_auth": [] }],
                    "responses": { "200": { "description": "创建成功" } }
                }
            },
            "/api/admin/dict/types/{id}": {
                "put": {
                    "tags": ["数据字典"], "summary": "更新字典类型",
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "security": [{ "bearer_auth": [] }],
                    "responses": { "200": { "description": "更新成功" } }
                },
                "delete": {
                    "tags": ["数据字典"], "summary": "删除字典类型",
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "security": [{ "bearer_auth": [] }],
                    "responses": { "200": { "description": "删除成功" } }
                }
            },
            "/api/admin/dict/{code}/items": {
                "get": {
                    "tags": ["数据字典"], "summary": "获取字典项",
                    "parameters": [{ "name": "code", "in": "path", "required": true, "schema": { "type": "string" } }],
                    "security": [{ "bearer_auth": [] }],
                    "responses": { "200": { "description": "字典项列表" } }
                }
            },
            "/api/admin/dict/items": {
                "get": {
                    "tags": ["数据字典"], "summary": "字典项列表（按类型）",
                    "parameters": [{ "name": "dict_type_id", "in": "query", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "security": [{ "bearer_auth": [] }],
                    "responses": { "200": { "description": "字典项列表" } }
                },
                "post": {
                    "tags": ["数据字典"], "summary": "新增字典项",
                    "security": [{ "bearer_auth": [] }],
                    "responses": { "200": { "description": "创建成功" } }
                }
            },
            "/api/admin/dict/items/{id}": {
                "put": {
                    "tags": ["数据字典"], "summary": "更新字典项",
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "security": [{ "bearer_auth": [] }],
                    "responses": { "200": { "description": "更新成功" } }
                },
                "delete": {
                    "tags": ["数据字典"], "summary": "删除字典项",
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "security": [{ "bearer_auth": [] }],
                    "responses": { "200": { "description": "删除成功" } }
                }
            },
            "/api/admin/dict/cached": {
                "get": {
                    "tags": ["数据字典"], "summary": "所有字典及项（缓存）",
                    "security": [{ "bearer_auth": [] }],
                    "responses": { "200": { "description": "字典数据" } }
                }
            },
            "/api/admin/dict/refresh": {
                "post": {
                    "tags": ["数据字典"], "summary": "刷新缓存",
                    "security": [{ "bearer_auth": [] }],
                    "responses": { "200": { "description": "缓存已刷新" } }
                }
            },
            // ── 系统监控 ──
            "/api/admin/monitor/system": {
                "get": {
                    "tags": ["系统监控"], "summary": "系统信息",
                    "security": [{ "bearer_auth": [] }],
                    "responses": { "200": { "description": "CPU/内存/磁盘/DB/Redis" } }
                }
            },
            "/api/admin/monitor/api-metrics": {
                "get": {
                    "tags": ["系统监控"], "summary": "API 接口指标",
                    "security": [{ "bearer_auth": [] }],
                    "responses": { "200": { "description": "接口性能指标" } }
                }
            },
            "/api/admin/monitor/alerts": {
                "get": {
                    "tags": ["系统监控"], "summary": "告警信息",
                    "security": [{ "bearer_auth": [] }],
                    "responses": { "200": { "description": "告警列表" } }
                }
            },
            "/api/admin/monitor/metrics/reset": {
                "post": {
                    "tags": ["系统监控"], "summary": "重置指标",
                    "security": [{ "bearer_auth": [] }],
                    "responses": { "200": { "description": "指标已重置" } }
                }
            },
            "/api/admin/monitor/system/export": {
                "get": {
                    "tags": ["系统监控"], "summary": "导出监控数据",
                    "security": [{ "bearer_auth": [] }],
                    "responses": { "200": { "description": "Excel 文件下载" } }
                }
            },
            // ── 操作日志 ──
            "/api/admin/audit-logs": {
                "get": {
                    "tags": ["系统"], "summary": "操作日志（分页）",
                    "security": [{ "bearer_auth": [] }],
                    "responses": { "200": { "description": "操作日志列表" } }
                }
            },
            "/api/admin/logs/audit/export": {
                "get": {
                    "tags": ["系统"], "summary": "导出操作日志",
                    "security": [{ "bearer_auth": [] }],
                    "responses": { "200": { "description": "Excel 文件下载" } }
                }
            },
            "/api/admin/export/users": {
                "get": {
                    "tags": ["系统"], "summary": "导出用户列表",
                    "security": [{ "bearer_auth": [] }],
                    "responses": { "200": { "description": "Excel 文件下载" } }
                }
            },
            "/api/admin/validate": {
                "post": {
                    "tags": ["系统"], "summary": "参数校验测试",
                    "security": [{ "bearer_auth": [] }],
                    "responses": { "200": { "description": "校验结果" } }
                }
            }
        },
        "components": {
            "securitySchemes": {
                "bearer_auth": {
                    "type": "http",
                    "scheme": "bearer",
                    "bearerFormat": "JWT"
                }
            },
            "schemas": {
                "ApiResponse_String": {
                    "type": "object",
                    "properties": {
                        "code": { "type": "integer" },
                        "message": { "type": "string" },
                        "data": { "type": "string", "nullable": true }
                    }
                },
                "ApiResponse_UserInfo": {
                    "type": "object",
                    "properties": {
                        "code": { "type": "integer" },
                        "message": { "type": "string" },
                        "data": { "$ref": "#/components/schemas/UserInfo" }
                    }
                },
                "ApiResponse_LoginResponse": {
                    "type": "object",
                    "properties": {
                        "code": { "type": "integer" },
                        "message": { "type": "string" },
                        "data": { "$ref": "#/components/schemas/LoginResponse" }
                    }
                },
                "LoginRequest": {
                    "type": "object",
                    "required": ["username", "password"],
                    "properties": {
                        "username": { "type": "string", "description": "用户名" },
                        "password": { "type": "string", "description": "密码" }
                    }
                },
                "LoginResponse": {
                    "type": "object",
                    "properties": {
                        "token": { "type": "string", "description": "JWT 令牌" },
                        "token_type": { "type": "string", "description": "令牌类型" }
                    }
                },
                "RegisterRequest": {
                    "type": "object",
                    "required": ["username", "email", "password"],
                    "properties": {
                        "username": { "type": "string" },
                        "email": { "type": "string", "format": "email" },
                        "password": { "type": "string", "minLength": 6 }
                    }
                },
                "UserInfo": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "format": "uuid" },
                        "username": { "type": "string" },
                        "email": { "type": "string" },
                        "role": { "type": "string" },
                        "is_active": { "type": "boolean" },
                        "created_at": { "type": "string", "format": "date-time" }
                    }
                },
                "UserManageRequest": {
                    "type": "object",
                    "required": ["username", "email", "role"],
                    "properties": {
                        "username": { "type": "string" },
                        "email": { "type": "string" },
                        "password": { "type": "string" },
                        "role": { "type": "string" },
                        "is_active": { "type": "boolean" }
                    }
                }
            }
        }
    })
}
