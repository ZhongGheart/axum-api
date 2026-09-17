//! OpenAPI 接口文档（由代码生成）
//!
//! 规范由 utoipa 从 handler 上的 `#[utoipa::path]` 与 DTO 上的 `ToSchema`
//! 派生生成，不再手写 JSON —— 文档与实现不会漂移。
//!
//! 新增接口时必须：
//! 1. 在 handler 上添加 `#[utoipa::path(...)]`
//! 2. 在下方 `paths(...)` 中登记该 handler
//! 3. 若引入新 DTO，确保其派生 `utoipa::ToSchema`（被 request_body/body 引用即自动登记）
//!
//! 端点：`GET /api/openapi.json`；Swagger UI：`GET /api/swagger-ui/index.html`

use axum::http::{header, Response, StatusCode};
use utoipa::OpenApi as _;

/// 由代码生成的 OpenAPI 文档
#[derive(utoipa::OpenApi)]
#[openapi(
    info(
        title = "Axum Admin API",
        version = "0.3.0",
        description = "基于 Axum + SQLx + JWT 的管理后台 API。规范由代码生成，与实现保持同步。",
    ),
    paths(
        crate::controller::auth::health,
        crate::controller::auth::register,
        crate::controller::auth::login,
        crate::controller::auth::me,
        crate::controller::auth::logout,
        crate::controller::rbac::admin_test,
        crate::controller::user::list_users,
        crate::controller::user::create_user,
        crate::controller::user::update_user,
        crate::controller::user::delete_user,
        crate::controller::user::batch_delete_users,
        crate::controller::user::toggle_user_status,
        crate::controller::user::reset_user_password,
        crate::controller::role::list_roles,
        crate::controller::role::create_role,
        crate::controller::role::update_role,
        crate::controller::role::delete_role,
        crate::controller::role::get_user_roles,
        crate::controller::role::assign_user_role,
        crate::controller::menu::list_menus,
        crate::controller::menu::create_menu,
        crate::controller::menu::update_menu,
        crate::controller::menu::delete_menu,
        crate::controller::menu::assign_role_menus,
        crate::controller::dict::list_types,
        crate::controller::dict::create_type,
        crate::controller::dict::update_type,
        crate::controller::dict::delete_type,
        crate::controller::dict::list_items,
        crate::controller::dict::create_item,
        crate::controller::dict::update_item,
        crate::controller::dict::delete_item,
        crate::controller::dict::get_items_by_code,
        crate::controller::dict::list_all_cached,
        crate::controller::dict::refresh_cache,
        crate::controller::demo::export_users,
        crate::controller::demo::validate_test,
        crate::controller::demo::list_audit_logs,
        crate::controller::demo::export_audit_logs,
        crate::controller::monitor::system_info,
        crate::controller::monitor::api_metrics,
        crate::controller::monitor::alerts,
        crate::controller::monitor::reset_metrics,
        crate::controller::monitor::export_system,
    )
)]
pub struct ApiDoc;

/// 构建 OpenAPI 文档（含 Bearer 安全方案）
///
/// utoipa 5.5 的 `#[openapi(components(...))]` 只接受 `schemas`/`responses`，
/// 安全方案需要在生成后以编程方式注入。
pub fn api_doc() -> utoipa::openapi::OpenApi {
    use utoipa::openapi::security::{Http, HttpAuthScheme, SecurityScheme};

    let mut doc = ApiDoc::openapi();
    let components = doc.components.get_or_insert_with(Default::default);

    let mut bearer = Http::new(HttpAuthScheme::Bearer);
    bearer.bearer_format = Some("JWT".to_string());
    bearer.description = Some("登录后取得的 JWT，放在 Authorization: Bearer <token>".to_string());
    components.add_security_scheme("bearer_auth", SecurityScheme::Http(bearer));

    doc
}

/// 生成 OpenAPI 规范 JSON
pub fn openapi_json() -> serde_json::Value {
    serde_json::to_value(api_doc()).expect("OpenAPI 规范序列化失败")
}

/// 列出文档中声明的所有路由 `(path, methods)`
///
/// 供集成测试校验"文档里声明的每条路由都真实存在"：
/// utoipa 的 `path = "..."` 是手写字符串，写错编译器不会发现，由该测试兜底。
pub fn documented_paths() -> Vec<(String, Vec<String>)> {
    let doc = api_doc();
    let mut routes: Vec<(String, Vec<String>)> = doc
        .paths
        .paths
        .iter()
        .map(|(path, item)| {
            let methods: Vec<String> = [
                ("GET", item.get.is_some()),
                ("PUT", item.put.is_some()),
                ("POST", item.post.is_some()),
                ("DELETE", item.delete.is_some()),
                ("PATCH", item.patch.is_some()),
                ("HEAD", item.head.is_some()),
                ("OPTIONS", item.options.is_some()),
                ("TRACE", item.trace.is_some()),
            ]
            .into_iter()
            .filter(|(_, present)| *present)
            .map(|(method, _): (&str, bool)| method.to_string())
            .collect();
            (path.clone(), methods)
        })
        .collect();
    routes.sort();
    routes
}

/// Swagger UI 页面
///
/// 指向同源的 `/api/openapi.json` 作为数据源。
/// 同源服务避免了 iframe 跨域限制，CDN 加载避免了前端打包体积膨胀。
pub async fn swagger_ui_handler(
    axum::extract::Path(path): axum::extract::Path<String>,
) -> Result<Response<String>, std::convert::Infallible> {
    if path != "index.html" && !path.is_empty() {
        return Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body("Not Found".to_string())
            .unwrap());
    }

    let html = r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
  <meta charset="UTF-8">
  <title>Axum Admin API - Swagger UI</title>
  <link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/swagger-ui-dist@5/swagger-ui.css">
  <style>
    html { box-sizing: border-box; overflow: -moz-scrollbars-vertical; overflow-y: scroll; }
    *, *:before, *:after { box-sizing: inherit; }
    body { margin: 0; background: #fafafa; }
    .topbar { display: none; }
  </style>
</head>
<body>
  <div id="swagger-ui"></div>
  <script src="https://cdn.jsdelivr.net/npm/swagger-ui-dist@5/swagger-ui-bundle.js"></script>
  <script>
    SwaggerUIBundle({
      url: '/api/openapi.json',
      dom_id: '#swagger-ui',
      deepLinking: true,
      presets: [
        SwaggerUIBundle.presets.apis,
        SwaggerUIBundle.SwaggerUIStandalonePreset
      ],
      plugins: [SwaggerUIBundle.plugins.DownloadUrl],
      layout: "StandaloneLayout",
      showExtensions: true,
      showCommonExtensions: true,
      tryItOutEnabled: true,
      defaultModelsExpandDepth: 3,
      defaultModelExpandDepth: 3,
    })
  </script>
</body>
</html>"#;

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .body(html.to_string())
        .unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 文档页面/规范端点自身不属于业务 API，不纳入覆盖检查
    const META_ENDPOINTS: [&str; 2] = ["/api/openapi.json", "/api/swagger-ui/{*path}"];

    /// 路由注册表里出现的路径必须全部同步到 OpenAPI 文档
    ///
    /// 这是反向检查：集成测试只覆盖"文档里有、实现里没有"，
    /// 该用例覆盖"实现里有、文档里没有"（新增接口忘写注解的典型漏项）。
    /// axum 没有路由自省 API，因此这里解析路由注册源码。
    #[test]
    fn every_registered_route_is_documented() {
        let router_src = include_str!("../router/mod.rs");
        let mut registered: Vec<String> = Vec::new();
        let mut rest = router_src;
        // 逐个 `.route(` 取紧随其后的字符串字面量；
        // 这样同时覆盖 `.route("/a", ..)` 与 `.route(\n "/a", ..)` 两种书写
        while let Some(idx) = rest.find(".route(") {
            rest = &rest[idx + ".route(".len()..];
            if let Some(quote) = rest.find('"') {
                let after = &rest[quote + 1..];
                if let Some(end) = after.find('"') {
                    registered.push(after[..end].to_string());
                }
            }
        }
        registered.retain(|path| !META_ENDPOINTS.contains(&path.as_str()));
        registered.sort();
        registered.dedup();

        assert!(!registered.is_empty(), "未解析到任何路由注册，检查解析逻辑");

        let documented: std::collections::HashSet<String> = documented_paths()
            .into_iter()
            .map(|(path, _)| path)
            .collect();
        let missing: Vec<&String> = registered
            .iter()
            .filter(|path| !documented.contains(*path))
            .collect();

        assert!(
            missing.is_empty(),
            "以下已注册路由未出现在 OpenAPI 文档中（请在 handler 上补 #[utoipa::path] 并登记到 ApiDoc）: {missing:?}"
        );
    }

    /// 规范必须包含 Bearer 安全方案，否则 Swagger UI 无法携带令牌调试
    #[test]
    fn security_scheme_is_present() {
        let doc = api_doc();
        let schemes = doc.components.expect("缺少 components").security_schemes;
        assert!(
            schemes.contains_key("bearer_auth"),
            "缺少 bearer_auth 安全方案"
        );
    }
}
