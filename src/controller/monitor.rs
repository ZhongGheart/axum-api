//! 系统监控控制器
//!
//! 提供系统信息、API 指标、数据库/Redis 状态等监控接口。
//! 所有接口仅 admin 角色可访问。

use axum::{extract::State, Json};

use crate::error::AppError;
use crate::model::ApiResponse;
use crate::router::AppState;
use crate::service::monitor::MonitorService;

/// GET /api/admin/monitor/system — 系统信息
pub async fn system_info(
    State(state): State<AppState>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    let sys = MonitorService::get_system_info()?;
    let db = MonitorService::get_database_status(&state).await?;
    let redis = MonitorService::get_redis_status(&state).await?;

    let result = serde_json::json!({
        "system": sys,
        "database": db,
        "redis": redis,
    });
    Ok(Json(ApiResponse::success(result)))
}

/// GET /api/admin/monitor/api-metrics — API 接口性能指标
pub async fn api_metrics(
    State(state): State<AppState>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    let metrics = state.metrics_collector;
    let snapshot = metrics.snapshot().await;
    // 计算聚合数据
    let total_calls: u64 = snapshot.iter().map(|m| m.call_count).sum();
    let total_errors: u64 = snapshot.iter().map(|m| m.error_count).sum();
    let avg_response: u64 = if total_calls > 0 {
        let total_duration: u64 = snapshot.iter().map(|m| m.total_duration_ms).sum();
        total_duration / total_calls
    } else {
        0
    };

    let result = serde_json::json!({
        "metrics": snapshot,
        "summary": {
            "total_endpoints": snapshot.len(),
            "total_calls": total_calls,
            "total_errors": total_errors,
            "error_rate": if total_calls > 0 {
                (total_errors as f64 / total_calls as f64 * 100.0 * 100.0).round() / 100.0
            } else {
                0.0
            },
            "avg_response_ms": avg_response,
        }
    });
    Ok(Json(ApiResponse::success(result)))
}

/// GET /api/admin/monitor/alerts — 告警信息
pub async fn alerts(
    State(state): State<AppState>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    let sys = MonitorService::get_system_info()?;
    let snapshot = state.metrics_collector.snapshot().await;
    let alert_list = MonitorService::check_alerts(&sys, &snapshot);

    let result = serde_json::json!({
        "alerts": alert_list,
        "alert_count": alert_list.len(),
        "critical_count": alert_list.iter().filter(|a| a.level == "critical").count(),
        "warning_count": alert_list.iter().filter(|a| a.level == "warning").count(),
    });
    Ok(Json(ApiResponse::success(result)))
}

/// POST /api/admin/monitor/metrics/reset — 重置指标
pub async fn reset_metrics(
    State(state): State<AppState>,
) -> Result<Json<ApiResponse<&'static str>>, AppError> {
    state.metrics_collector.reset().await;
    Ok(Json(ApiResponse::success("指标已重置")))
}

/// GET /api/admin/monitor/system/export — 导出系统监控数据
pub async fn export_system(
    State(state): State<AppState>,
) -> Result<axum::response::Response, AppError> {
    use crate::utils::export::{ExcelExport, ExcelColumn};
    use std::sync::Arc;

    let sys = MonitorService::get_system_info()?;
    let db = MonitorService::get_database_status(&state).await?;
    let redis = MonitorService::get_redis_status(&state).await?;

    let mut export = ExcelExport::new("系统监控.xlsx");

    // 系统资源 sheet
    let columns = vec![
        ExcelColumn { header: "指标".into(), width: 20.0 },
        ExcelColumn { header: "值".into(), width: 20.0 },
    ];
    let rows = vec![
        vec!["操作系统".into(), sys.os.clone()],
        vec!["主机名".into(), sys.hostname.clone()],
        vec!["CPU 使用率".into(), format!("{:.1}%", sys.cpu.usage_percent)],
        vec!["CPU 核心数".into(), sys.cpu.core_count.to_string()],
        vec!["内存总量".into(), format!("{} MB", sys.memory.total_mb)],
        vec!["内存已用".into(), format!("{} MB", sys.memory.used_mb)],
        vec!["内存使用率".into(), format!("{:.1}%", sys.memory.usage_percent)],
        vec!["数据库活跃连接".into(), db.active_connections.to_string()],
        vec!["Redis 连接".into(), redis.connected_clients.to_string()],
    ];
    export.add_sheet_from_rows("系统概览", &columns, &rows)?;

    // 磁盘 sheet
    let disk_columns = vec![
        ExcelColumn { header: "名称".into(), width: 15.0 },
        ExcelColumn { header: "总量(GB)".into(), width: 12.0 },
        ExcelColumn { header: "已用(GB)".into(), width: 12.0 },
        ExcelColumn { header: "使用率".into(), width: 10.0 },
    ];
    let disk_rows: Vec<Vec<String>> = sys.disks.iter().map(|d| {
        vec![
            d.name.clone(),
            d.total_gb.to_string(),
            d.used_gb.to_string(),
            format!("{:.1}%", d.usage_percent),
        ]
    }).collect();
    export.add_sheet_from_rows("磁盘信息", &disk_columns, &disk_rows)?;

    export.into_response()
}
