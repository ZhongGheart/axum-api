//! RBAC 权限控制器
//!
//! 提供需要特定角色权限才能访问的测试接口。

use axum::{extract::State, Json};

use crate::error::AppError;
use crate::middleware::auth::AuthenticatedUser;
use crate::model::ApiResponse;
use crate::router::AppState;

/// GET /api/admin/test — 管理员权限测试
///
/// 需要 `admin` 角色才能访问（通过中间件拦截）。
#[utoipa::path(
    get,
    path = "/api/admin/test",
    tag = "系统",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "管理员权限校验通过", body = ApiResponse<String>))
)]
pub async fn admin_test(
    _state: State<AppState>,
    auth_user: AuthenticatedUser,
) -> Result<Json<ApiResponse<String>>, AppError> {
    let msg = format!(
        "管理员访问成功！用户: {}, 角色: {:?}",
        auth_user.user_id, auth_user.roles
    );
    Ok(Json(ApiResponse::success(msg)))
}
