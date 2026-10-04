//! 部门服务层
//!
//! 职责边界：**仓储负责存，服务负责判断取值合不合法**。
//!
//! 与 `service::setting` 同构：服务层做校验与装配，仓储层只做 CRUD。
//!
//! ── 为什么移动节点是独立操作 ────────────────────────────────
//! 移动节点需要循环防护（不能把一个节点移动到自己的子孙下面），
//! 而修改名称不需要。把两者混在一个 `update` 里会让校验逻辑变得复杂，
//! 且"改个名字"和"移动整个子树"是两种完全不同的操作，
//! 前者高频低频判断，后者低频高判断。

use uuid::Uuid;

use crate::error::AppError;
use crate::model::department::{
    build_tree, CreateDepartmentRequest, Department, DepartmentFlat, DepartmentNode,
    DepartmentUser, MoveDepartmentRequest, UpdateDepartmentRequest,
};
use crate::repository::department::DepartmentRepository;

/// 部门服务
#[derive(Debug, Clone)]
pub struct DepartmentService {
    repo: DepartmentRepository,
}

impl DepartmentService {
    pub fn new(repo: DepartmentRepository) -> Self {
        Self { repo }
    }

    pub fn repo(&self) -> &DepartmentRepository {
        &self.repo
    }

    /// 部门树
    ///
    /// 用应用层 [`build_tree`] 构建，与菜单树同一模式。
    pub async fn tree(&self) -> Result<Vec<DepartmentNode>, AppError> {
        let all = self.repo.list_all().await?;
        Ok(build_tree(&all))
    }

    /// 扁平列表（用于下拉选择）
    ///
    /// 与 [`Self::tree`] 分开是因为下拉选择需要 `level` 和 `path`，
    /// 而树形返回需要 `children`。两者服务不同的 UI 场景。
    pub async fn flat_list(&self) -> Result<Vec<DepartmentFlat>, AppError> {
        let all = self.repo.list_all().await?;
        let mut out = Vec::new();
        for d in &all {
            let (level, path) = self.compute_level_and_path(d, &all);
            out.push(DepartmentFlat {
                id: d.id,
                parent_id: d.parent_id,
                name: d.name.clone(),
                level,
                path,
            });
        }
        Ok(out)
    }

    /// 计算某部门的层级深度与路径
    ///
    /// 从当前节点向上追溯到根，收集路径上的名称。
    /// 环的处理：如果数据里存在环，`visited` 集合会检测到并停止追溯，
    /// 返回当前已收集的路径（可能不完整，但不会死循环）。
    fn compute_level_and_path(&self, dept: &Department, all: &[Department]) -> (i32, String) {
        let mut path = vec![dept.name.clone()];
        let mut current = dept.parent_id;
        let mut visited = std::collections::HashSet::new();
        visited.insert(dept.id);

        while let Some(pid) = current {
            if !visited.insert(pid) {
                // 环：停止追溯
                break;
            }
            match all.iter().find(|d| d.id == pid) {
                Some(parent) => {
                    path.push(parent.name.clone());
                    current = parent.parent_id;
                }
                None => break,
            }
        }

        path.reverse();
        let level = (path.len() as i32) - 1;
        (level, path.join("/"))
    }

    /// 新建部门
    ///
    /// 校验：
    /// 1. `parent_id` 不为 `None` 时必须存在
    /// 2. `name` 长度 1-100
    /// 3. 同一父部门下名称唯一
    pub async fn create(&self, req: &CreateDepartmentRequest) -> Result<Department, AppError> {
        let name = req.name.trim();
        if name.is_empty() || name.len() > 100 {
            return Err(AppError::BadRequest(
                "部门名称长度必须在 1-100 个字符之间".to_string(),
            ));
        }

        if let Some(pid) = req.parent_id {
            if self.repo.find_by_id(pid).await?.is_none() {
                return Err(AppError::BadRequest("父部门不存在".to_string()));
            }
        }

        // 同一父部门下名称唯一
        let siblings = self.repo.list_all().await?;
        if siblings
            .iter()
            .any(|d| d.parent_id == req.parent_id && d.name == name)
        {
            return Err(AppError::Conflict(format!(
                "同一父部门下已存在名为 \"{name}\" 的部门"
            )));
        }

        let sort_order = req.sort_order.unwrap_or(0);
        self.repo
            .create(req.parent_id, name, req.description.as_deref(), sort_order)
            .await
    }

    /// 修改部门（名称 / 描述 / 排序）
    ///
    /// **不修改 `parent_id`**：移动节点是独立操作（[`Self::move`]）。
    pub async fn update(
        &self,
        id: Uuid,
        req: &UpdateDepartmentRequest,
    ) -> Result<Department, AppError> {
        let existing = self
            .repo
            .find_by_id(id)
            .await?
            .ok_or_else(|| AppError::NotFound("部门不存在".to_string()))?;

        let name = match &req.name {
            Some(n) => {
                let n = n.trim();
                if n.is_empty() || n.len() > 100 {
                    return Err(AppError::BadRequest(
                        "部门名称长度必须在 1-100 个字符之间".to_string(),
                    ));
                }
                // 同一父部门下名称唯一（排除自己）
                let siblings = self.repo.list_all().await?;
                if siblings
                    .iter()
                    .any(|d| d.id != id && d.parent_id == existing.parent_id && d.name == n)
                {
                    return Err(AppError::Conflict(format!(
                        "同一父部门下已存在名为 \"{n}\" 的部门"
                    )));
                }
                n.to_string()
            }
            None => existing.name,
        };

        let description = match &req.description {
            Some(Some(d)) => Some(d.as_str()),
            Some(None) => None,
            None => existing.description.as_deref(),
        };

        let sort_order = req.sort_order.unwrap_or(existing.sort_order);

        self.repo.update(id, &name, description, sort_order).await
    }

    /// 移动部门
    ///
    /// 方法名刻意不叫 `move`：那是 Rust 关键字，作为方法名需要 `r#` 转义，
    /// 而转义后的名字在调用点读起来像拼写错误。
    ///
    /// 循环防护：
    /// 1. 不能移动到自己下面（`new_parent_id == id`）
    /// 2. 不能移动到自己的子孙下面（`is_descendant`）
    pub async fn move_to(
        &self,
        id: Uuid,
        req: &MoveDepartmentRequest,
    ) -> Result<Department, AppError> {
        // 只检查存在性，不需要读取字段
        if self.repo.find_by_id(id).await?.is_none() {
            return Err(AppError::NotFound("部门不存在".to_string()));
        }

        let new_parent_id = req.new_parent_id;

        // 不能移动到自己下面
        if new_parent_id == Some(id) {
            return Err(AppError::BadRequest("不能把部门移动到自己下面".to_string()));
        }

        // 新父部门必须存在
        if let Some(pid) = new_parent_id {
            if self.repo.find_by_id(pid).await?.is_none() {
                return Err(AppError::BadRequest("目标父部门不存在".to_string()));
            }
        }

        // 不能移动到自己的子孙下面
        if let Some(pid) = new_parent_id {
            if self.repo.is_descendant(id, pid).await? {
                return Err(AppError::BadRequest(
                    "不能把部门移动到自己的子孙下面".to_string(),
                ));
            }
        }

        self.repo.set_parent(id, new_parent_id).await
    }

    /// 删除部门
    ///
    /// 有子部门时拒绝删除，提示"请先移动或删除子部门"。
    /// `ON DELETE RESTRICT` 会在数据库层面阻止，但服务层先检查一次
    /// 能给出可操作的错误提示。
    pub async fn delete(&self, id: Uuid) -> Result<(), AppError> {
        let existing = self
            .repo
            .find_by_id(id)
            .await?
            .ok_or_else(|| AppError::NotFound("部门不存在".to_string()))?;

        let children = self.repo.count_children(id).await?;
        if children > 0 {
            return Err(AppError::BadRequest(format!(
                "该部门下还有 {children} 个子部门，请先移动或删除它们"
            )));
        }

        // 有用户时不拒绝删除，但给出警告：用户的 dept_id 会变成 NULL
        let user_count = self.repo.count_users(id).await?;
        if user_count > 0 {
            tracing::warn!(
                "删除部门 {}（{}），{} 个用户的 dept_id 将变成 NULL",
                existing.name,
                id,
                user_count
            );
        }

        self.repo.delete(id).await
    }

    /// 列出某部门下的用户
    pub async fn list_users(&self, id: Uuid) -> Result<Vec<DepartmentUser>, AppError> {
        // **刻意不查部门是否存在**，与 `GET /api/admin/users/{id}/sessions` 保持一致：
        // 那里对不存在的用户返回 200 + 空数组，这里若回 404，同一个 `{id}` 在
        // 两个"查这个人的附属信息"的端点上就给出两种相反的答案。
        //
        // 代价要认：一个写错或已删除的 id 与"这个人确实没在线"确实长得一样。
        // 换来的是这个端点遵守仓库对读端点的统一约定（不存在即空列表），
        // `every_documented_endpoint_is_reachable_without_a_server_error`
        // 会拿一个**不存在**的 UUID 探针，读端点回 4xx 一律算缺陷。
        //
        // 真的需要区分时，调用方手上已经有部门树（`GET /api/admin/departments`），
        // 不必靠这个端点去反推 id 是否有效。
        self.repo.list_users(id).await
    }
}
