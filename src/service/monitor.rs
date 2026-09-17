//! 监控服务层
//!
//! 提供系统信息收集、告警逻辑

use std::time::Duration;
use sysinfo::{Disks, System};

use crate::error::AppError;
use crate::router::AppState;

/// CPU 负载信息
#[derive(Debug, serde::Serialize)]
pub struct CpuInfo {
    pub usage_percent: f32,
    pub core_count: usize,
    pub frequency_mhz: u64,
}

/// 内存信息
#[derive(Debug, serde::Serialize)]
pub struct MemoryInfo {
    pub total_mb: u64,
    pub used_mb: u64,
    pub free_mb: u64,
    pub usage_percent: f32,
}

/// 磁盘信息
#[derive(Debug, serde::Serialize)]
pub struct DiskInfo {
    pub total_gb: u64,
    pub used_gb: u64,
    pub free_gb: u64,
    pub usage_percent: f32,
    pub name: String,
}

/// 系统信息
#[derive(Debug, serde::Serialize)]
pub struct SystemInfo {
    pub os: String,
    pub hostname: String,
    pub kernel: String,
    pub uptime_seconds: u64,
    pub cpu: CpuInfo,
    pub memory: MemoryInfo,
    pub disks: Vec<DiskInfo>,
}

/// 数据库状态
#[derive(Debug, serde::Serialize)]
pub struct DatabaseStatus {
    pub connected: bool,
    pub active_connections: i32,
    pub max_connections: i32,
    pub idle_connections: i32,
    pub database_size_mb: f64,
}

/// Redis 状态
#[derive(Debug, serde::Serialize)]
pub struct RedisStatus {
    pub connected: bool,
    pub uptime_seconds: u64,
    pub used_memory_bytes: u64,
    pub connected_clients: u64,
    pub total_commands_processed: u64,
}

/// 监控服务
pub struct MonitorService;

impl MonitorService {
    /// 收集系统信息
    pub fn get_system_info() -> Result<SystemInfo, AppError> {
        let mut sys = System::new();
        sys.refresh_all();
        std::thread::sleep(Duration::from_millis(200));
        sys.refresh_cpu_usage();

        let cpu = CpuInfo {
            usage_percent: sys.global_cpu_usage(),
            core_count: sys.physical_core_count().unwrap_or(0),
            frequency_mhz: 0, // 0.33 不直接暴露频率
        };

        let mem = MemoryInfo {
            total_mb: sys.total_memory() / 1024 / 1024,
            used_mb: sys.used_memory() / 1024 / 1024,
            free_mb: sys.free_memory() / 1024 / 1024,
            usage_percent: if sys.total_memory() > 0 {
                (sys.used_memory() as f32 / sys.total_memory() as f32) * 100.0
            } else {
                0.0
            },
        };

        let disks: Vec<DiskInfo> = {
            let disks_list = Disks::new_with_refreshed_list();
            disks_list
                .iter()
                .map(|disk| {
                    let total = disk.total_space();
                    let available = disk.available_space();
                    let used = total.saturating_sub(available);
                    DiskInfo {
                        total_gb: total / 1024 / 1024 / 1024,
                        used_gb: used / 1024 / 1024 / 1024,
                        free_gb: available / 1024 / 1024 / 1024,
                        usage_percent: if total > 0 {
                            (used as f32 / total as f32) * 100.0
                        } else {
                            0.0
                        },
                        name: disk.name().to_str().unwrap_or("").to_string(),
                    }
                })
                .collect()
        };

        Ok(SystemInfo {
            os: System::long_os_version().unwrap_or_default(),
            hostname: System::host_name().unwrap_or_default(),
            kernel: System::kernel_version().unwrap_or_default(),
            uptime_seconds: System::uptime(),
            cpu,
            memory: mem,
            disks,
        })
    }

    /// 获取数据库状态
    pub async fn get_database_status(state: &AppState) -> Result<DatabaseStatus, AppError> {
        let pool = state.db_pool.writer();
        let row: Result<(i32, i32, f64), _> = sqlx::query_as(
            "SELECT numbackends, (SELECT setting::int FROM pg_settings WHERE name = 'max_connections') as max_conn, \
             pg_database_size(current_database())::float8 / 1024 / 1024 as db_size_mb \
             FROM pg_stat_database WHERE datname = current_database()"
        )
        .fetch_one(pool)
        .await;

        match row {
            Ok((active, max_conn, db_size)) => Ok(DatabaseStatus {
                connected: true,
                active_connections: active,
                max_connections: max_conn,
                idle_connections: max_conn.saturating_sub(active),
                database_size_mb: db_size,
            }),
            Err(e) => {
                tracing::warn!("查询数据库状态失败: {e}");
                Ok(DatabaseStatus {
                    connected: false,
                    active_connections: 0,
                    max_connections: 0,
                    idle_connections: 0,
                    database_size_mb: 0.0,
                })
            }
        }
    }

    /// 获取 Redis 状态
    pub async fn get_redis_status(state: &AppState) -> Result<RedisStatus, AppError> {
        let mut conn = state.redis_client.conn.clone();
        let info = redis::cmd("INFO").query_async::<String>(&mut conn).await;
        match info {
            Ok(info_str) => {
                let uptime = parse_redis_info(&info_str, "uptime_in_seconds");
                let mem = parse_redis_info(&info_str, "used_memory");
                let clients = parse_redis_info(&info_str, "connected_clients");
                let cmds = parse_redis_info(&info_str, "total_commands_processed");
                Ok(RedisStatus {
                    connected: true,
                    uptime_seconds: uptime,
                    used_memory_bytes: mem,
                    connected_clients: clients,
                    total_commands_processed: cmds,
                })
            }
            Err(e) => {
                tracing::warn!("查询 Redis 状态失败: {e}");
                Ok(RedisStatus {
                    connected: false,
                    uptime_seconds: 0,
                    used_memory_bytes: 0,
                    connected_clients: 0,
                    total_commands_processed: 0,
                })
            }
        }
    }

    /// 告警检查
    pub fn check_alerts(
        system: &SystemInfo,
        metrics: &[crate::middleware::api_metrics::EndpointMetric],
    ) -> Vec<AlertInfo> {
        let mut alerts = vec![];

        // CPU 过高告警
        if system.cpu.usage_percent > 90.0 {
            alerts.push(AlertInfo {
                level: "critical".into(),
                message: format!("CPU 使用率过高: {:.1}%", system.cpu.usage_percent),
            });
        } else if system.cpu.usage_percent > 75.0 {
            alerts.push(AlertInfo {
                level: "warning".into(),
                message: format!("CPU 使用率偏高: {:.1}%", system.cpu.usage_percent),
            });
        }

        // 内存告警
        if system.memory.usage_percent > 90.0 {
            alerts.push(AlertInfo {
                level: "critical".into(),
                message: format!("内存使用率过高: {:.1}%", system.memory.usage_percent),
            });
        } else if system.memory.usage_percent > 80.0 {
            alerts.push(AlertInfo {
                level: "warning".into(),
                message: format!("内存使用率偏高: {:.1}%", system.memory.usage_percent),
            });
        }

        // 磁盘告警
        for disk in &system.disks {
            if disk.usage_percent > 90.0 {
                alerts.push(AlertInfo {
                    level: "critical".into(),
                    message: format!("磁盘 {} 使用率: {:.1}%", disk.name, disk.usage_percent),
                });
            }
        }

        // 接口报错率告警
        for m in metrics {
            if m.call_count > 10 {
                let error_rate = m.error_count as f32 / m.call_count as f32 * 100.0;
                if error_rate > 20.0 {
                    alerts.push(AlertInfo {
                        level: "warning".into(),
                        message: format!("{} {} 报错率: {:.1}%", m.method, m.path, error_rate),
                    });
                }
            }
        }

        // 输出告警日志
        for a in &alerts {
            match a.level.as_str() {
                "critical" => tracing::error!("[ALERT] {}: {}", a.level, a.message),
                "warning" => tracing::warn!("[ALERT] {}: {}", a.level, a.message),
                _ => tracing::info!("[ALERT] {}: {}", a.level, a.message),
            }
        }

        alerts
    }
}

#[derive(Debug, serde::Serialize)]
pub struct AlertInfo {
    pub level: String,
    pub message: String,
}

/// 解析 Redis INFO 命令返回的键值对
fn parse_redis_info(info: &str, key: &str) -> u64 {
    for line in info.lines() {
        if line.starts_with(key) {
            if let Some(value) = line.split(':').nth(1) {
                return value.trim().parse().unwrap_or(0);
            }
        }
    }
    0
}
