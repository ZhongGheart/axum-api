//! 后端能力测试控制器
//!
//! 提供分页查询、Excel 导出、参数校验等接口的演示。

use axum::{extract::{Query, State}, Json};
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::model::ApiResponse;
use crate::router::AppState;
use crate::utils::pagination::{PaginationParams, PaginatedResponse};
use crate::utils::export::{ExcelExport, ExcelColumn};
use crate::utils::validation;

/// 校验测试请求
#[derive(Debug, Deserialize)]
pub struct ValidateTestRequest {
    pub username: String,
    pub email: Option<String>,
}

/// 校验测试响应
#[derive(Debug, Serialize)]
pub struct ValidateTestResponse {
    pub username_valid: bool,
    pub username_message: String,
    pub email_valid: Option<bool>,
    pub email_message: Option<String>,
}

/// GET /api/admin/export/users — 导出用户列表（Excel）
pub async fn export_users(
    State(state): State<AppState>,
) -> Result<axum::response::Response, AppError> {
    // 查询所有用户
    let users = sqlx::query_as::<_, crate::model::User>(
        "SELECT id, username, email, password_hash, role, is_active, created_at, updated_at FROM users ORDER BY created_at DESC"
    )
    .fetch_all(&state.auth_service.user_repo.pool)
    .await
    .map_err(|e| AppError::InternalServerError(format!("查询用户失败: {e}")))?;

    let columns = vec![
        ExcelColumn { header: "用户名".into(), width: 20.0 },
        ExcelColumn { header: "邮箱".into(), width: 30.0 },
        ExcelColumn { header: "角色".into(), width: 15.0 },
        ExcelColumn { header: "是否激活".into(), width: 15.0 },
        ExcelColumn { header: "创建时间".into(), width: 25.0 },
    ];

    let mut export = ExcelExport::new("用户列表.xlsx");
    export.add_sheet_from_rows("用户列表", &columns, &users.iter().map(|u| {
        vec![
            u.username.clone(),
            u.email.clone(),
            u.role.to_string(),
            if u.is_active { "是".into() } else { "否".into() },
            u.created_at.format("%Y-%m-%d %H:%M:%S").to_string(),
        ]
    }).collect::<Vec<_>>())?;

    export.into_response()
}

/// POST /api/admin/validate — 参数校验演示
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

/// GET /api/admin/audit-logs — 查询操作日志（分页）
pub async fn list_audit_logs(
    State(state): State<AppState>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<ApiResponse<PaginatedResponse<crate::model::AuditLog>>>, AppError> {
    use crate::service::crud::CrudTemplate;

    let result = CrudTemplate::paginate::<crate::model::AuditLog>(
        &state.auth_service.user_repo.pool,
        "audit_logs",
        &params,
        &["created_at", "username", "action", "status_code"],
        None,
    ).await?;

    Ok(Json(ApiResponse::success(result)))
}
