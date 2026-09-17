//! 后端能力测试控制器
//!
//! 提供分页查询、Excel 导出、参数校验等接口的演示。

use axum::{
    extract::{Query, State},
    Json,
};
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::model::ApiResponse;
use crate::router::AppState;
use crate::utils::export::{ExcelColumn, ExcelExport};
use crate::utils::pagination::{PaginatedResponse, PaginationParams};
use crate::utils::validation;

/// 校验测试请求
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ValidateTestRequest {
    pub username: String,
    pub email: Option<String>,
}

/// 校验测试响应
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ValidateTestResponse {
    pub username_valid: bool,
    pub username_message: String,
    pub email_valid: Option<bool>,
    pub email_message: Option<String>,
}

/// GET /api/admin/export/users — 导出用户列表（Excel）
#[utoipa::path(
    get,
    path = "/api/admin/export/users",
    tag = "导出",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "用户列表 Excel 文件（二进制）"))
)]
pub async fn export_users(
    State(state): State<AppState>,
) -> Result<axum::response::Response, AppError> {
    // 查询所有用户（角色来自 user_roles）
    let users = sqlx::query_as::<_, crate::model::User>(
        "SELECT id, username, email, password_hash, is_active, created_at, updated_at          FROM users ORDER BY created_at DESC",
    )
    .fetch_all(state.auth_service.user_repo.pool())
    .await
    .map_err(|e| AppError::InternalServerError(format!("查询用户失败: {e}")))?;

    let ids: Vec<uuid::Uuid> = users.iter().map(|u| u.id).collect();
    let mut roles_by_user: std::collections::HashMap<uuid::Uuid, Vec<String>> =
        std::collections::HashMap::new();
    for (user_id, role) in state
        .auth_service
        .user_repo
        .find_roles_for_users(&ids)
        .await?
    {
        roles_by_user.entry(user_id).or_default().push(role);
    }

    let columns = vec![
        ExcelColumn {
            header: "用户名".into(),
            width: 20.0,
        },
        ExcelColumn {
            header: "邮箱".into(),
            width: 30.0,
        },
        ExcelColumn {
            header: "角色".into(),
            width: 15.0,
        },
        ExcelColumn {
            header: "是否激活".into(),
            width: 15.0,
        },
        ExcelColumn {
            header: "创建时间".into(),
            width: 25.0,
        },
    ];

    let mut export = ExcelExport::new("用户列表.xlsx");
    export.add_sheet_from_rows(
        "用户列表",
        &columns,
        &users
            .iter()
            .map(|u| {
                vec![
                    u.username.clone(),
                    u.email.clone(),
                    roles_by_user
                        .get(&u.id)
                        .map(|roles| roles.join(","))
                        .unwrap_or_default(),
                    if u.is_active {
                        "是".into()
                    } else {
                        "否".into()
                    },
                    u.created_at.format("%Y-%m-%d %H:%M:%S").to_string(),
                ]
            })
            .collect::<Vec<_>>(),
    )?;

    export.into_response()
}

/// POST /api/admin/validate — 参数校验演示
#[utoipa::path(
    post,
    path = "/api/admin/validate",
    tag = "系统",
    security(("bearer_auth" = [])),
    request_body = ValidateTestRequest,
    responses((status = 200, description = "校验结果", body = ApiResponse<ValidateTestResponse>))
)]
pub async fn validate_test(
    Json(req): Json<ValidateTestRequest>,
) -> Result<Json<ApiResponse<ValidateTestResponse>>, AppError> {
    let mut resp = ValidateTestResponse {
        username_valid: true,
        username_message: String::new(),
        email_valid: None,
        email_message: None,
    };

    // 校验用户名
    if let Err(e) = validation::validate_username(&req.username) {
        resp.username_valid = false;
        resp.username_message = e.to_string();
    } else {
        resp.username_message = "校验通过".into();
    }

    // 校验邮箱（可选）
    if let Some(email) = &req.email {
        let mut valid = true;
        if let Err(e) = validation::validate_email(email) {
            valid = false;
            resp.email_message = Some(e.to_string());
        } else {
            resp.email_message = Some("校验通过".into());
        }
        resp.email_valid = Some(valid);
    }

    Ok(Json(ApiResponse::success(resp)))
}

/// GET /api/admin/logs/audit/export — 导出操作日志（Excel）
#[utoipa::path(
    get,
    path = "/api/admin/logs/audit/export",
    tag = "导出",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "操作日志 Excel 文件（二进制）"))
)]
pub async fn export_audit_logs(
    State(state): State<AppState>,
) -> Result<axum::response::Response, AppError> {
    let logs: Vec<crate::model::AuditLog> = sqlx::query_as(
        "SELECT id, user_id, username, action, method, path, params, result, status_code, client_ip, duration_ms, created_at FROM audit_logs ORDER BY created_at DESC LIMIT 10000"
    )
    .fetch_all(state.auth_service.user_repo.pool())
    .await
    .map_err(|e| AppError::InternalServerError(format!("查询日志失败: {e}")))?;

    let columns = vec![
        ExcelColumn {
            header: "用户名".into(),
            width: 15.0,
        },
        ExcelColumn {
            header: "操作".into(),
            width: 25.0,
        },
        ExcelColumn {
            header: "方法".into(),
            width: 10.0,
        },
        ExcelColumn {
            header: "路径".into(),
            width: 40.0,
        },
        ExcelColumn {
            header: "状态码".into(),
            width: 10.0,
        },
        ExcelColumn {
            header: "IP".into(),
            width: 20.0,
        },
        ExcelColumn {
            header: "耗时(ms)".into(),
            width: 12.0,
        },
        ExcelColumn {
            header: "时间".into(),
            width: 25.0,
        },
    ];

    let mut export = ExcelExport::new("操作日志.xlsx");
    export.add_sheet_from_rows(
        "操作日志",
        &columns,
        &logs
            .iter()
            .map(|l| {
                vec![
                    l.username.clone().unwrap_or_default(),
                    l.action.clone(),
                    l.method.clone(),
                    l.path.clone(),
                    l.status_code.map(|s| s.to_string()).unwrap_or_default(),
                    l.client_ip.clone().unwrap_or_default(),
                    l.duration_ms.map(|d| d.to_string()).unwrap_or_default(),
                    l.created_at.format("%Y-%m-%d %H:%M:%S").to_string(),
                ]
            })
            .collect::<Vec<_>>(),
    )?;

    export.into_response()
}

/// GET /api/admin/audit-logs — 查询操作日志（分页）
#[utoipa::path(
    get,
    path = "/api/admin/audit-logs",
    tag = "操作日志",
    security(("bearer_auth" = [])),
    params(
        ("page" = Option<i64>, Query, description = "页码"),
        ("page_size" = Option<i64>, Query, description = "每页条数"),
        ("sort_by" = Option<String>, Query, description = "排序字段"),
        ("sort_order" = Option<String>, Query, description = "排序方向 asc/desc"),
    ),
    responses((status = 200, description = "操作日志分页", body = ApiResponse<PaginatedResponse<crate::model::AuditLog>>))
)]
pub async fn list_audit_logs(
    State(state): State<AppState>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<ApiResponse<PaginatedResponse<crate::model::AuditLog>>>, AppError> {
    let result = state.audit_log_repo.paginate(&params).await?;

    Ok(Json(ApiResponse::success(result)))
}
