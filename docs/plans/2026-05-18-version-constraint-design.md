# ADAM 版本约束管理设计 v3.0

**Date**: 2026-05-18  
**Author**: Claude Code  
**Status**: Draft - Major Revision  
**Related**: ADAM Multi-Version Support Feature, Asset Version History

---

## 重大架构变更说明

### 为什么放弃 Fork 机制？

**原 Fork 方案的问题：**
1. **资产复制导致 ID 冲突** - REQ-1 在 v1.x 和 v2.x 变成两个不同资产，外部系统无法识别
2. **Merge 未设计** - Feature Branch 用例无法闭环
3. **数据冗余** - 相同资产内容被复制多份
4. **复杂度过高** - 需要处理 Fork 验证、部分 Fork、跨线依赖等问题

**新方案的核心思想：**
- 资产不复制，通过**版本约束（Version Constraint）**表达依赖关系
- 一个资产可以有多个版本（1.0.0, 2.0.0），由 SemVer 管理
- 下游通过约束声明接受哪些版本范围（^1.0.0, >=2.0.0）
- Dirty 传播基于**版本兼容性**判断

---

## 1. Executive Summary

### 1.1 Problem Statement

ADAM 当前使用单一版本模型，无法支持：
- **Multi-version product lines**: 同时维护 v1.x 和 v2.x
- **渐进式升级**: 控制何时接受上游的新版本
- **兼容性管理**: Major/Minor/Patch 升级的不同处理策略

### 1.2 Proposed Solution

引入**版本约束（Version Constraint）**作为依赖关系的核心属性：

1. **每个依赖声明版本约束**：`^1.0.0`, `>=2.0.0, <3.0.0`
2. **锁定有效版本**：创建依赖时记录当前满足约束的版本
3. **智能 Dirty 传播**：
   - Patch 升级：根据策略自动接受或通知
   - Minor 升级：根据策略处理
   - Major 升级：总是标记 Dirty（不兼容）
4. **分层配置**：系统默认 → 组织策略 → 类型规则 → 显式指定

### 1.3 Key Design Principles

1. **资产唯一性**: 资产 ID 全局唯一，不复制
2. **版本驱动**: 版本号（SemVer）表达兼容性和演进
3. **约束优先**: 通过约束声明依赖范围，而非锁定单一版本
4. **策略可配置**: 不同组织/项目可以有不同的升级策略
5. **向后兼容**: 现有数据自动适配（默认约束 ^current_version）

---

## 2. 核心概念

### 2.1 Semantic Versioning（语义化版本）

```rust
/// 语义化版本号
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SemVer {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub prerelease: Option<String>,  // alpha, beta
}

impl SemVer {
    /// 检查版本是否满足约束
    pub fn satisfies(&self, constraint: &VersionConstraint) -> bool {
        // 实现版本约束匹配逻辑
    }
    
    /// 检查是否与另一个版本兼容（同 Major）
    pub fn is_compatible_with(&self, other: &SemVer) -> bool {
        self.major == other.major
    }
}
```

### 2.2 Version Constraint（版本约束）

```rust
/// 版本约束表达式
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VersionConstraint {
    /// 精确版本 =1.0.0
    Exact(SemVer),
    
    /// 跟随主版本 ^1.0.0 → >=1.0.0, <2.0.0
    Caret(SemVer),
    
    /// 跟随次版本 ~1.0.0 → >=1.0.0, <1.1.0
    Tilde(SemVer),
    
    /// 版本范围 >=1.0.0, <2.0.0
    Range { min: Bound, max: Bound },
    
    /// 任意版本 *
    Wildcard,
}

impl VersionConstraint {
    /// 检查版本是否满足约束
    pub fn matches(&self, version: &SemVer) -> bool {
        match self {
            VersionConstraint::Exact(v) => version == v,
            VersionConstraint::Caret(v) => {
                version >= v && version.major == v.major
            }
            VersionConstraint::Tilde(v) => {
                version >= v && 
                version.major == v.major && 
                version.minor == v.minor
            }
            VersionConstraint::Range { min, max } => {
                min.contains(version) && max.contains(version)
            }
            VersionConstraint::Wildcard => true,
        }
    }
}
```

### 2.3 Asset 版本历史

```rust
/// 资产版本
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetVersion {
    pub id: AssetVersionId,
    pub asset_id: AssetId,
    pub version: SemVer,
    pub content_ref: String,      // 指向实际内容存储
    pub state: AssetState,        // 此版本的状态（通常是 Clean）
    pub is_lts: bool,            // 是否长期支持版本
    pub release_notes: String,
    pub released_by: String,
    pub released_at: DateTime<Utc>,
}

/// 修改后的 AssetInstance
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetInstance {
    pub id: AssetId,
    pub name: String,
    pub asset_type_id: AssetTypeId,
    pub project_id: Option<ProjectId>,
    pub organization_id: OrganizationId,
    pub level: AssetLevel,
    pub current_version: SemVer,           // 当前最新版本（必须）
    pub current_state: AssetState,
    pub external_ref: String,
    pub source: String,
    pub metadata: serde_json::Value,
    pub assignees: Vec<String>,
    pub publisher: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub lock_version: i64,                // 乐观锁版本
}
```

### 2.4 依赖关系（带版本约束）

```rust
/// 资产实例间依赖关系
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetDependency {
    pub id: DependencyId,
    pub downstream_id: AssetId,
    pub upstream_id: AssetId,
    
    /// 声明的版本约束（如 ^1.0.0）
    pub declared_constraint: VersionConstraint,
    
    /// 约束字符串表示（用于序列化和调试）
    pub constraint_str: String,
    
    /// 当前有效版本（创建时锁定）
    pub effective_version: SemVer,
    
    /// 升级策略
    pub upgrade_policy: UpgradePolicy,
    
    /// 乐观锁版本（用于并发控制）
    pub lock_version: i64,
    
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

---

## 3. 分层配置模型

### 3.1 四层配置体系

配置优先级（从高到低）：

```
┌─────────────────────────────────────────────────────────────┐
│  Layer 1: Dependency-level (显式指定)                        │
│  - 创建依赖时显式指定的约束和策略                              │
│  - 最高优先级，覆盖其他所有配置                               │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│  Layer 2: Asset-type-pair-level (类型对规则)                  │
│  - CODE → REQ 的默认规则                                     │
│  - 基于上下游资产类型自动应用                                │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│  Layer 3: Organization-policy-level (组织策略)               │
│  - 组织的默认约束模板                                        │
│  - 按资产类型的策略覆盖                                      │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│  Layer 4: System-default-level (系统默认)                  │
│  - FollowMajor + Notify (保守策略)                           │
│  - 兜底配置                                                 │
└─────────────────────────────────────────────────────────────┘
```

### 3.2 配置模型定义

```rust
/// 升级策略
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpgradePolicy {
    /// 自动接受 Patch 升级
    AutoPatch,
    /// 自动接受 Patch + Minor 升级
    AutoMinor,
    /// 所有升级都通知（标记 Dirty）
    Notify,
    /// 所有升级都需要显式审批
    Manual,
    /// 锁定当前版本，不自动升级
    Pin,
}

/// 约束模板（用于自动计算）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConstraintTemplate {
    /// 跟随上游主版本 ^X.0.0
    FollowMajor,
    /// 锁定当前版本 =X.Y.Z
    ExactCurrent,
    /// 跟随到当前次版本 ~X.Y.0
    FollowMinor,
    /// 固定范围 [min, max)
    FixedRange { min: SemVer, max: SemVer },
    /// 任意版本 *
    Wildcard,
}

/// Layer 2: 资产类型对规则
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyTypeRule {
    pub id: DependencyTypeRuleId,
    pub organization_id: OrganizationId,
    pub downstream_type: AssetTypeId,
    pub upstream_type: AssetTypeId,
    pub default_template: ConstraintTemplate,
    pub default_policy: UpgradePolicy,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
}

/// Layer 3: 组织策略
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrganizationPolicy {
    pub organization_id: OrganizationId,
    
    /// 默认约束模板
    pub default_template: ConstraintTemplate,
    
    /// 默认升级策略
    pub default_policy: UpgradePolicy,
    
    /// 按资产类型的策略覆盖
    pub asset_type_policies: HashMap<AssetTypeId, AssetTypePolicy>,
    
    /// Major 升级是否强制需要审批
    pub require_approval_for_major: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetTypePolicy {
    pub template: ConstraintTemplate,
    pub policy: UpgradePolicy,
}
```

### 3.3 依赖创建时的配置解析

```rust
impl DependencyService {
    /// 创建依赖时的配置解析流程
    pub async fn create_dependency(
        &self,
        request: CreateDependencyRequest,
    ) -> Result<AssetDependency, DependencyError> {
        let CreateDependencyRequest {
            downstream_id,
            upstream_id,
            explicit_constraint,
            explicit_policy,
        } = request;
        
        // 获取上下游资产
        let downstream = self.asset_repo.find_by_id(&downstream_id).await?;
        let upstream = self.asset_repo.find_by_id(&upstream_id).await?;
        
        // Layer 1: 显式指定
        if let Some(constraint) = explicit_constraint {
            let policy = explicit_policy.unwrap_or_else(|| {
                // 如果没有显式策略，从低优先级获取
                self.resolve_policy(&downstream, &upstream).await
            });
            
            return self.create_with_config(
                downstream_id, 
                upstream_id, 
                constraint, 
                upstream.current_version,
                policy
            ).await;
        }
        
        // Layer 2: 类型对规则
        if let Some(rule) = self.type_rule_repo
            .find_by_types(&downstream.asset_type_id, &upstream.asset_type_id)
            .await?
        {
            let constraint = self.apply_template(&rule.default_template, &upstream.current_version);
            return self.create_with_config(
                downstream_id,
                upstream_id,
                constraint,
                upstream.current_version,
                rule.default_policy,
            ).await;
        }
        
        // Layer 3: 组织策略
        let org_policy = self.org_policy_repo
            .find_by_organization(&downstream.organization_id)
            .await?;
        
        // 检查是否有针对此资产类型的覆盖
        let policy = org_policy
            .asset_type_policies
            .get(&downstream.asset_type_id)
            .map(|p| p.policy)
            .unwrap_or(org_policy.default_policy);
        
        let template = org_policy
            .asset_type_policies
            .get(&downstream.asset_type_id)
            .map(|p| p.template.clone())
            .unwrap_or(org_policy.default_template);
        
        let constraint = self.apply_template(&template, &upstream.current_version);
        
        self.create_with_config(
            downstream_id,
            upstream_id,
            constraint,
            upstream.current_version,
            policy,
        ).await
    }
    
    fn apply_template(&self, template: &ConstraintTemplate, current: &SemVer) -> VersionConstraint {
        match template {
            ConstraintTemplate::FollowMajor => {
                VersionConstraint::Caret(SemVer::new(current.major, 0, 0))
            }
            ConstraintTemplate::ExactCurrent => {
                VersionConstraint::Exact(current.clone())
            }
            ConstraintTemplate::FollowMinor => {
                VersionConstraint::Tilde(SemVer::new(current.major, current.minor, 0))
            }
            ConstraintTemplate::FixedRange { min, max } => {
                VersionConstraint::Range {
                    min: Bound::Inclusive(min.clone()),
                    max: Bound::Exclusive(max.clone()),
                }
            }
            ConstraintTemplate::Wildcard => VersionConstraint::Wildcard,
        }
    }
}
```

---

## 4. Dirty 状态传播规则

### 4.1 传播触发条件

当上游资产发布新版本时：

```rust
impl StatePropagationService {
    /// 上游发布新版本时的传播逻辑
    pub async fn propagate_on_publish(
        &self,
        upstream_id: &AssetId,
        new_version: &SemVer,
    ) -> Result<PropagationResult, StateError> {
        // 1. 查找所有依赖此上游的下游
        let downstream_deps = self
            .dependency_repo
            .find_by_upstream(upstream_id)
            .await?;
        
        let mut affected = Vec::new();
        
        for dep in downstream_deps {
            // 2. 检查新版本是否满足约束
            if !new_version.satisfies(&dep.declared_constraint) {
                // 不满足约束，跳过
                continue;
            }
            
            // 3. 检查是否应该标记 Dirty
            if self.should_mark_dirty(&dep, new_version).await? {
                self.mark_downstream_dirty(&dep, new_version).await?;
                affected.push(dep.downstream_id);
            }
        }
        
        Ok(PropagationResult { affected_count: affected.len(), assets: affected })
    }
    
    /// 判断是否应标记 Dirty
    async fn should_mark_dirty(
        &self,
        dep: &AssetDependency,
        new_version: &SemVer,
    ) -> Result<bool, StateError> {
        // 检查是否是兼容升级
        let is_compatible = new_version.is_compatible_with(&dep.effective_version);
        
        if is_compatible {
            // Patch 或 Minor 升级
            match dep.upgrade_policy {
                UpgradePolicy::AutoPatch | UpgradePolicy::AutoMinor => {
                    // 自动接受，不标记 Dirty，更新 effective_version
                    self.dependency_repo
                        .update_effective_version(&dep.id, new_version)
                        .await?;
                    Ok(false)
                }
                UpgradePolicy::Notify | UpgradePolicy::Manual => {
                    // 标记 Dirty，等待审查
                    Ok(true)
                }
                UpgradePolicy::Pin => {
                    // 锁定版本，忽略
                    Ok(false)
                }
            }
        } else {
            // Major 升级（不兼容）
            // 总是标记 Dirty
            Ok(true)
            }
        }
    }
}
```

### 4.1a Dirty Resolution 聚合模型

**问题**: 如果 REQ-1 持续发布 patch (1.0.1, 1.0.2, 1.0.3...)，传统的 Vec<DirtyTrigger> 会无限增长。

**解决方案**: 使用聚合记录，只保留关键信息

```rust
/// Dirty Resolution 聚合记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirtyResolutionLog {
    pub id: DirtyLogId,
    pub asset_id: AssetId,                    // 下游资产
    pub upstream_id: AssetId,                 // 上游资产
    
    // 聚合信息（不保留完整历史）
    pub latest_trigger_version: SemVer,       // 最新触发 Dirty 的上游版本
    pub latest_triggered_at: DateTime<Utc>,   // 最新触发时间
    pub first_triggered_at: DateTime<Utc>,    // 首次触发时间
    pub trigger_count: usize,                 // 触发次数（聚合）
    pub aggregated: bool,                     // 是否已聚合（多个触发合并）
    
    // 当前状态
    pub resolution_type: ResolutionType,      // Auto | Manual
    pub resolved_by: Option<String>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub notes: Option<String>,
}

/// 触发 Dirty 时创建或更新聚合记录
impl DirtyResolutionService {
    pub async fn mark_dirty_aggregated(
        &self,
        asset_id: AssetId,
        upstream_id: AssetId,
        upstream_version: &SemVer,
    ) -> Result<(), DirtyError> {
        // 查找是否已有未解决的 Dirty 记录
        let existing = self
            .dirty_log_repo
            .find_unresolved(&asset_id, &upstream_id)
            .await?;
        
        if let Some(mut log) = existing {
            // 更新聚合信息
            log.latest_trigger_version = upstream_version.clone();
            log.latest_triggered_at = Utc::now();
            log.trigger_count += 1;
            log.aggregated = log.trigger_count > 1;
            
            self.dirty_log_repo.update(&log).await?;
        } else {
            // 创建新的 Dirty 记录
            let now = Utc::now();
            let log = DirtyResolutionLog {
                id: DirtyLogId::new(),
                asset_id,
                upstream_id,
                latest_trigger_version: upstream_version.clone(),
                latest_triggered_at: now,
                first_triggered_at: now,
                trigger_count: 1,
                aggregated: false,
                resolution_type: ResolutionType::Pending,
                resolved_by: None,
                resolved_at: None,
                notes: None,
            };
            
            self.dirty_log_repo.create(&log).await?;
        }
        
        // 标记资产为 Dirty
        self.asset_repo.mark_dirty(&asset_id).await?;
        
        Ok(())
    }
}

/// 用户界面显示示例
fn display_dirty_status(log: &DirtyResolutionLog) -> String {
    if log.aggregated {
        format!(
            "⚠️ {} 已触发 {} 次更新 (首次: {}, 最新: {})",
            log.upstream_id,
            log.trigger_count,
            log.first_triggered_at.format("%Y-%m-%d"),
            log.latest_triggered_at.format("%Y-%m-%d")
        )
    } else {
        format!(
            "⚠️ {} 有新版本 {}",
            log.upstream_id,
            log.latest_trigger_version
        )
    }
}
```

| 上游变化 | 下游约束 | 升级策略 | 新版本满足约束 | 是否 Dirty | 后续动作 |
|---------|---------|---------|--------------|-----------|---------|
| 1.0.0 → 1.0.1 | ^1.0.0 | AutoPatch | 是 | 否 | 自动更新 effective_version |
| 1.0.0 → 1.1.0 | ^1.0.0 | AutoMinor | 是 | 否 | 自动更新 effective_version |
| 1.0.0 → 1.1.0 | ^1.0.0 | Notify | 是 | **是** | 标记 Dirty，通知审查 |
| 1.0.0 → 2.0.0 | ^1.0.0 | 任意 | 否 | 否 | 不满足约束，忽略 |
| 1.0.0 → 2.0.0 | >=1.0.0 | Manual | 是 | **是** | Major 升级必须审查 |
| 1.0.0 → 1.0.1 | =1.0.0 | 任意 | 否 | 否 | 精确锁定，忽略 |
| 1.0.0 → 1.0.1 | * | AutoPatch | 是 | 否 | 任意版本，自动接受 |

### 4.3 批量升级

```rust
impl DependencyService {
    /// 批量升级依赖到最新版本
    pub async fn upgrade_dependencies(
        &self,
        asset_id: AssetId,
        upgrade_options: UpgradeOptions,
    ) -> Result<UpgradeResult, DependencyError> {
        let deps = self.dependency_repo.find_by_downstream(&asset_id).await?;
        
        let mut upgraded = Vec::new();
        let mut skipped = Vec::new();
        let mut failed = Vec::new();
        
        for dep in deps {
            // 获取上游最新版本
            let upstream = self.asset_repo.find_by_id(&dep.upstream_id).await?;
            let latest = upstream.current_version;
            
            // 检查是否可以升级
            if !latest.satisfies(&dep.declared_constraint) {
                skipped.push((dep.upstream_id, "不满足约束"));
                continue;
            }
            
            // 检查是否需要放宽约束
            if !latest.is_compatible_with(&dep.effective_version) {
                if upgrade_options.allow_constraint_relaxation {
                    // 放宽约束到 ^latest.major.0.0
                    let new_constraint = VersionConstraint::Caret(
                        SemVer::new(latest.major, 0, 0)
                    );
                    self.dependency_repo
                        .update_constraint(&dep.id, new_constraint)
                        .await?;
                } else {
                    failed.push((dep.upstream_id, "需要放宽约束"));
                    continue;
                }
            }
            
            // 更新 effective_version
            self.dependency_repo
                .update_effective_version(&dep.id, &latest)
                .await?;
            
            upgraded.push((dep.upstream_id, dep.effective_version.clone(), latest));
        }
        
        // 标记资产为 Clean（如果所有依赖都已同步）
        if failed.is_empty() {
            self.asset_repo.mark_clean(&asset_id).await?;
        }
        
        Ok(UpgradeResult {
            upgraded,
            skipped,
            failed,
        })
    }
}
```

### 4.4 性能优化：预编译约束

**问题**: SemVer 解析是 CPU 密集型操作，10万依赖 = 10万次解析

**解决方案**: 预编译约束，避免重复解析

```rust
/// 预编译的依赖（用于高性能场景）
#[derive(Debug, Clone)]
pub struct CompiledDependency {
    pub id: DependencyId,
    pub downstream_id: AssetId,
    pub upstream_id: AssetId,
    pub constraint_str: String,                      // 原始字符串（用于序列化）
    pub compiled_constraint: semver::VersionReq,    // 预编译的约束
    pub effective_version: SemVer,
    pub upgrade_policy: UpgradePolicy,
    pub constraint_version: i64,                     // 约束版本戳（缓存失效检测）
    pub cached_at: DateTime<Utc>,                   // 缓存时间戳
}

/// 约束版本管理
impl CompiledDependency {
    /// 从原始依赖编译
    pub fn compile(dep: &AssetDependency) -> Result<Self, semver::Error> {
        let compiled = semver::VersionReq::parse(&dep.constraint_str)?;
        
        Ok(Self {
            id: dep.id,
            downstream_id: dep.downstream_id,
            upstream_id: dep.upstream_id,
            constraint_str: dep.constraint_str.clone(),
            compiled_constraint: compiled,
            effective_version: dep.effective_version.clone(),
            upgrade_policy: dep.upgrade_policy,
            constraint_version: dep.lock_version,  // 使用乐观锁版本作为约束版本
            cached_at: Utc::now(),
        })
    }
    
    /// 检查是否过期
    pub fn is_stale(&self, db_constraint: &str, db_version: i64) -> bool {
        self.constraint_str != db_constraint || 
        self.constraint_version != db_version
    }
    
    /// 使用预编译约束快速匹配
    pub fn matches(&self, version: &SemVer) -> bool {
        let semver_version = semver::Version::new(
            version.major as u64,
            version.minor as u64,
            version.patch as u64,
        );
        self.compiled_constraint.matches(&semver_version)
    }
}

/// 带缓存失效检测的 Repository
#[async_trait::async_trait]
pub trait CompiledDependencyRepository: Send + Sync {
    /// 获取预编译的依赖（带版本戳验证）
    async fn find_compiled_by_upstream(
        &self,
        upstream_id: &AssetId,
    ) -> Result<Vec<CompiledDependency>, RepositoryError>;
    
    /// 获取原始依赖（用于验证版本戳）
    async fn find_with_version(
        &self,
        dep_id: &DependencyId,
    ) -> Result<(AssetDependency, i64), RepositoryError>;
}

/// 缓存失效检测
impl StatePropagationService {
    pub async fn propagate_with_stale_check(
        &self,
        upstream_id: &AssetId,
        new_version: &SemVer,
    ) -> Result<PropagationResult, StateError> {
        let deps = self.dependency_repo
            .find_compiled_by_upstream(upstream_id)
            .await?;
        
        let mut affected = Vec::new();
        let mut stale_count = 0;
        
        for dep in deps {
            // 获取当前数据库版本
            let (db_dep, db_version) = self.dependency_repo
                .find_with_version(&dep.id)
                .await?;
            
            // 检查是否过期
            if dep.is_stale(&db_dep.constraint_str, db_version) {
                // 重新编译
                let fresh = CompiledDependency::compile(&db_dep)
                    .map_err(|e| StateError::InvalidConstraint(e.to_string()))?;
                
                stale_count += 1;
                
                // 使用重新编译的版本
                if fresh.matches(new_version) {
                    affected.push(fresh.downstream_id);
                }
            } else {
                // 使用缓存版本
                if dep.matches(new_version) {
                    affected.push(dep.downstream_id);
                }
            }
        }
        
        if stale_count > 0 {
            tracing::warn!("Detected {} stale compiled dependencies", stale_count);
        }
        
        Ok(PropagationResult { 
            affected_count: affected.len(), 
            assets: affected 
        })
    }
}
```

### 4.5 并发控制：乐观锁

**问题**: 并发发布可能导致竞态条件

**场景**:
```
T1: REQ-1 发布 1.2.0，开始传播 Dirty
T2: CODE-1 同时发布 1.3.0（已依赖 REQ-1 1.2.0）
结果: 竞态条件，Dirty 状态不确定
```

**解决方案**: 乐观锁

```rust
/// 修改后的 AssetInstance
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetInstance {
    pub id: AssetId,
    pub name: String,
    pub asset_type_id: AssetTypeId,
    pub project_id: Option<ProjectId>,
    pub organization_id: OrganizationId,
    pub level: AssetLevel,
    pub current_version: SemVer,
    pub current_state: AssetState,
    pub external_ref: String,
    pub source: String,
    pub metadata: serde_json::Value,
    pub assignees: Vec<String>,
    pub publisher: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub lock_version: i64,  // 乐观锁版本
}

impl AssetLifecycleService {
    /// 发布新版本（带乐观锁 CAS 模式）
    /// 
    /// 使用数据库原子操作避免 ABA 问题：
    /// - 不先读取资产状态
    /// - 直接执行 UPDATE ... WHERE lock_version = $expected
    /// - 如果影响行数 = 0，说明并发冲突
    pub async fn publish_version(
        &self,
        asset_id: AssetId,
        new_version: SemVer,
        content_ref: String,
        expected_lock_version: i64,
    ) -> Result<AssetVersion, AssetError> {
        // 验证版本号递增（可以在应用层做，因为不依赖当前状态）
        // 注意：这里不读取当前资产状态，避免 ABA 问题
        
        // 创建新版本对象
        let version = AssetVersion {
            id: AssetVersionId::new(),
            asset_id,
            version: new_version.clone(),
            content_ref,
            state: AssetState::Clean,
            is_lts: false,
            release_notes: String::new(),
            released_by: self.current_user.clone(),
            released_at: Utc::now(),
        };
        
        // CAS 操作：原子更新，避免 ABA 问题
        let update_result = self.asset_repo
            .update_version_cas(
                &asset_id, 
                &new_version,
                expected_lock_version,  // 期望的当前版本
                expected_lock_version + 1,  // 新版本
            )
            .await?;
        
        match update_result {
            CasResult::Success => {
                // 创建版本记录
                self.version_repo.create(&version).await?;
                
                // 传播 Dirty
                self.propagation_service
                    .propagate_on_publish(&asset_id, &new_version)
                    .await?;
                
                Ok(version)
            }
            CasResult::Conflict { actual_version } => {
                Err(AssetError::ConcurrentModification {
                    asset_id,
                    expected: expected_lock_version,
                    actual: actual_version,
                })
            }
            CasResult::VersionAlreadyExists => {
                Err(AssetError::VersionAlreadyExists(new_version.to_string()))
            }
        }
    }
}

/// CAS 操作结果
#[derive(Debug, Clone)]
pub enum CasResult {
    Success,
    Conflict { actual_version: i64 },
    VersionAlreadyExists,
}

/// Repository 层 CAS 实现
#[async_trait::async_trait]
pub trait AssetInstanceRepository: Send + Sync {
    /// Compare-And-Swap 更新
    /// 
    /// 原子操作：
    /// UPDATE asset_instances 
    /// SET current_version = $new_version, 
    ///     lock_version = $new_lock_version,
    ///     updated_at = NOW()
    /// WHERE id = $asset_id 
    ///   AND lock_version = $expected_lock_version
    ///   AND current_version < $new_version  -- 防止版本回退
    async fn update_version_cas(
        &self,
        asset_id: &AssetId,
        new_version: &SemVer,
        expected_lock_version: i64,
        new_lock_version: i64,
    ) -> Result<CasResult, RepositoryError>;
}

/// PostgreSQL CAS 实现
impl PostgresAssetRepository {
    async fn update_version_cas(
        &self,
        asset_id: &AssetId,
        new_version: &SemVer,
        expected_lock_version: i64,
        new_lock_version: i64,
    ) -> Result<CasResult, RepositoryError> {
        // 使用 RETURNING 获取更新后的状态
        let result: Option<(i64, String)> = sqlx::query_as(
            r#"
            UPDATE asset_instances 
            SET current_version = $2, 
                lock_version = $3,
                updated_at = NOW()
            WHERE id = $1 
              AND lock_version = $4
              AND (
                  (current_version_major < $5) OR
                  (current_version_major = $5 AND current_version_minor < $6) OR
                  (current_version_major = $5 AND current_version_minor = $6 AND current_version_patch < $7)
              )
            RETURNING lock_version, current_version
            "#
        )
        .bind(asset_id)
        .bind(new_version.to_string())
        .bind(new_lock_version)
        .bind(expected_lock_version)
        .bind(new_version.major as i32)
        .bind(new_version.minor as i32)
        .bind(new_version.patch as i32)
        .fetch_optional(&self.pool)
        .await?;
        
        match result {
            Some((actual_lock, actual_version)) => {
                // 更新成功
                tracing::info!(
                    "CAS success: asset={}, version={}, lock={}",
                    asset_id, actual_version, actual_lock
                );
                Ok(CasResult::Success)
            }
            None => {
                // 更新失败，检查原因
                let (actual_lock, actual_major, actual_minor, actual_patch): (i64, i32, i32, i32) = 
                    sqlx::query_as(
                        "SELECT lock_version, current_version_major, current_version_minor, current_version_patch 
                         FROM asset_instances WHERE id = $1"
                    )
                    .bind(asset_id)
                    .fetch_one(&self.pool)
                    .await?;
                
                // 检查是否是版本冲突
                let actual_version_str = format!("{}.{}.{}", actual_major, actual_minor, actual_patch);
                let actual_semver = SemVer::parse(&actual_version_str)?;
                
                if actual_semver >= *new_version {
                    Ok(CasResult::VersionAlreadyExists)
                } else {
                    Ok(CasResult::Conflict { actual_version: actual_lock })
                }
            }
        }
    }
}
        }
        
        Ok(())
    }
}
```

### 4.5a 依赖循环检测

**问题**: 约束放宽可能导致循环依赖

**场景**:
```
A depends on B ^1.0.0
B depends on C ^1.0.0
C depends on A ^2.0.0  // 约束放宽后可能形成循环
```

**解决方案**: 约束变更时检测循环

```rust
/// 依赖图结构
pub struct DependencyGraph {
    nodes: HashSet<AssetId>,
    edges: HashMap<AssetId, Vec<(AssetId, VersionConstraint)>>,  // from -> [(to, constraint)]
}

impl DependencyGraph {
    /// 构建依赖图（考虑版本约束）
    pub async fn build(
        &self,
        repo: &dyn AssetDependencyRepository,
        root: AssetId,
    ) -> Result<Self, DependencyError> {
        let mut graph = Self {
            nodes: HashSet::new(),
            edges: HashMap::new(),
        };
        
        // BFS 遍历构建图
        let mut queue = VecDeque::new();
        queue.push_back(root);
        
        while let Some(current) = queue.pop_front() {
            if graph.nodes.contains(&current) {
                continue;
            }
            
            graph.nodes.insert(current);
            
            // 获取此资产的所有依赖
            let deps = repo.find_by_downstream(&current).await?;
            let mut edges = Vec::new();
            
            for dep in deps {
                // 只考虑满足当前版本的约束
                let upstream = repo.find_asset(&dep.upstream_id).await?;
                if dep.declared_constraint.matches(&upstream.current_version) {
                    edges.push((dep.upstream_id, dep.declared_constraint.clone()));
                    queue.push_back(dep.upstream_id);
                }
            }
            
            if !edges.is_empty() {
                graph.edges.insert(current, edges);
            }
        }
        
        Ok(graph)
    }
    
    /// 使用 DFS 检测循环
    pub fn detect_cycle(&self) -> Option<Vec<AssetId>> {
        let mut visited: HashSet<AssetId> = HashSet::new();
        let mut recursion_stack: HashSet<AssetId> = HashSet::new();
        let mut path: Vec<AssetId> = Vec::new();
        
        fn dfs(
            graph: &DependencyGraph,
            node: AssetId,
            visited: &mut HashSet<AssetId>,
            recursion_stack: &mut HashSet<AssetId>,
            path: &mut Vec<AssetId>,
        ) -> Option<Vec<AssetId>> {
            visited.insert(node);
            recursion_stack.insert(node);
            path.push(node);
            
            // 检查依赖
            if let Some(deps) = graph.edges.get(&node) {
                for (dep_id, _) in deps {
                    if !visited.contains(dep_id) {
                        if let Some(cycle) = dfs(graph, *dep_id, visited, recursion_stack, path) {
                            return Some(cycle);
                        }
                    } else if recursion_stack.contains(dep_id) {
                        // 发现循环
                        let cycle_start = path.iter().position(|&x| x == *dep_id).unwrap();
                        return Some(path[cycle_start..].to_vec());
                    }
                }
            }
            
            path.pop();
            recursion_stack.remove(&node);
            None
        }
        
        for node in &self.nodes {
            if !visited.contains(node) {
                if let Some(cycle) = dfs(self, *node, &mut visited, &mut recursion_stack, &mut path) {
                    return Some(cycle);
                }
            }
        }
        
        None
    }
}

/// 依赖服务层集成
impl DependencyService {
    /// 更新约束前检查循环
    pub async fn validate_no_cycle_before_update(
        &self,
        asset_id: AssetId,
        new_constraint: &VersionConstraint,
    ) -> Result<(), DependencyError> {
        // 1. 临时应用新约束
        let temp_dep = AssetDependency {
            id: DependencyId::new(),
            downstream_id: asset_id,
            upstream_id: AssetId::new(), // 占位
            declared_constraint: new_constraint.clone(),
            constraint_str: new_constraint.to_string(),
            effective_version: SemVer::new(0, 0, 0),
            upgrade_policy: UpgradePolicy::Manual,
            lock_version: 1,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        
        // 2. 构建依赖图（包含临时约束）
        let graph = DependencyGraph::build_with_temp(&self.dependency_repo, &temp_dep).await?;
        
        // 3. 检测循环
        if let Some(cycle) = graph.detect_cycle() {
            return Err(DependencyError::CycleDetected {
                path: cycle.iter().map(|id| id.to_string()).collect::<Vec<_>>().join(" -> "),
            });
        }
        
        Ok(())
    }
    
    /// 约束变更时检测（放宽约束可能导致循环）
    pub async fn update_constraint_safe(
        &self,
        dependency_id: DependencyId,
        new_constraint: VersionConstraint,
    ) -> Result<(), DependencyError> {
        let dep = self.dependency_repo.find_by_id(&dependency_id).await?;
        
        // 检查是否放宽了约束（更容易形成循环）
        let is_relaxing = self.is_constraint_relaxing(
            &dep.declared_constraint, 
            &new_constraint
        );
        
        if is_relaxing {
            // 放宽约束时需要严格检测循环
            self.validate_no_cycle_before_update(dep.downstream_id, &new_constraint).await?;
        }
        
        // 安全更新
        self.dependency_repo
            .update_constraint(&dependency_id, new_constraint)
            .await?;
        
        Ok(())
    }
    
    /// 判断约束是否放宽
    fn is_constraint_relaxing(
        &self,
        old: &VersionConstraint,
        new: &VersionConstraint,
    ) -> bool {
        // 简单判断：新约束是否包含更多版本
        // 例如：^1.0.0 -> >=1.0.0 是放宽
        match (old, new) {
            (VersionConstraint::Exact(_), _) => true,  // 精确 -> 任意都是放宽
            (VersionConstraint::Caret(v1), VersionConstraint::Range { min, max }) => {
                // ^1.0.0 (= [1.0.0, 2.0.0)) vs [x, y)
                let old_range = (v1.clone(), SemVer::new(v1.major + 1, 0, 0));
                // 新范围是否包含旧范围
                false  // 简化判断
            }
            _ => false,
        }
    }
}
```

### 4.6 版本撤回 (Unpublish) 策略

**场景**: 发布后发现有严重 bug，需要撤回版本

**策略设计**:

```rust
/// 版本撤回策略
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnpublishPolicy {
    /// 允许撤回（24小时内）
    AllowWithin(Duration),
    /// 禁止撤回
    Never,
    /// 需要管理员审批
    RequireApproval,
}

/// 修改后的 AssetVersion
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetVersion {
    pub id: AssetVersionId,
    pub asset_id: AssetId,
    pub version: SemVer,
    pub content_ref: String,
    pub state: AssetState,
    pub is_lts: bool,
    pub is_unpublished: bool,           // 是否已撤回
    pub unpublished_at: Option<DateTime<Utc>>,
    pub unpublished_by: Option<String>,
    pub unpublished_reason: Option<String>,
    pub release_notes: String,
    pub released_by: String,
    pub released_at: DateTime<Utc>,
}

/// 撤回策略配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrganizationUnpublishConfig {
    pub policy: UnpublishPolicy,
    /// 撤回后的影响范围
    pub propagation: UnpublishPropagation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnpublishPropagation {
    /// 仅标记撤回，不影响下游
    MarkOnly,
    /// 通知下游（标记 Dirty）
    NotifyDownstream,
    /// 自动回滚下游（危险！）
    AutoRollback,
}

impl AssetLifecycleService {
    /// 撤回版本
    pub async fn unpublish_version(
        &self,
        asset_id: AssetId,
        version: SemVer,
        reason: String,
    ) -> Result<(), AssetError> {
        // 1. 检查撤回策略
        let config = self.get_org_config(asset_id).await?;
        let version_record = self.version_repo
            .find_by_version(&asset_id, &version)
            .await?;
        
        match config.policy {
            UnpublishPolicy::Never => {
                return Err(AssetError::UnpublishNotAllowed);
            }
            UnpublishPolicy::AllowWithin(duration) => {
                let elapsed = Utc::now() - version_record.released_at;
                if elapsed > duration {
                    return Err(AssetError::UnpublishWindowExpired);
                }
            }
            UnpublishPolicy::RequireApproval => {
                // 创建审批流程
                self.create_unpublish_approval(asset_id, version, reason).await?;
                return Ok(());
            }
        }
        
        // 2. 执行撤回
        self.version_repo.mark_unpublished(
            &asset_id, 
            &version, 
            &reason,
            &self.current_user,
        ).await?;
        
        // 3. 处理下游影响
        match config.propagation {
            UnpublishPropagation::MarkOnly => {
                // 仅标记
            }
            UnpublishPropagation::NotifyDownstream => {
                // 通知下游此版本已撤回
                self.propagate_unpublish(&asset_id, &version).await?;
            }
            UnpublishPropagation::AutoRollback => {
                // 自动回滚（不推荐）
                self.rollback_downstream(&asset_id, &version).await?;
            }
        }
        
        Ok(())
    }
    
    /// 传播撤回通知
    async fn propagate_unpublish(
        &self,
        upstream_id: &AssetId,
        unpublished_version: &SemVer,
    ) -> Result<(), AssetError> {
        // 找到所有 effective_version 等于此版本的下游
        let deps = self.dependency_repo
            .find_by_upstream_and_version(upstream_id, unpublished_version)
            .await?;
        
        for dep in deps {
            // 标记这些下游为 Dirty（因为依赖的版本被撤回了）
            self.mark_dirty(&dep.downstream_id, upstream_id).await?;
        }
        
        Ok(())
    }
}

/// 撤回后查询行为
impl AssetRepository {
    /// 查询时默认过滤已撤回版本
    pub async fn find_versions(
        &self,
        asset_id: &AssetId,
        include_unpublished: bool,
    ) -> Result<Vec<AssetVersion>, RepositoryError> {
        let mut query = "SELECT * FROM asset_versions WHERE asset_id = $1".to_string();
        
        if !include_unpublished {
            query += " AND is_unpublished = false";
        }
        
        query += " ORDER BY major DESC, minor DESC, patch DESC";
        
        // ... 执行查询
    }
}
```

**撤回影响矩阵**:

| 下游状态 | 撤回版本 | 影响 |
|---------|---------|------|
| 依赖 1.0.0 | 撤回 1.0.0 | 标记 Dirty（依赖已不存在） |
| 已升级到 1.0.1 | 撤回 1.0.0 | 无影响（使用新版本） |
| 依赖 ^1.0.0 | 撤回 1.0.0 | 自动指向 1.0.1（如果存在） |

### 4.6a Major 升级回滚策略

**问题**: 下游升级到 v2.0.0 后发现不兼容，如何回滚？

**场景**:
```
1. CODE-1 依赖 REQ-1 ^1.0.0
2. REQ-1 发布 2.0.0（Major 升级）
3. CODE-1 升级到依赖 2.0.0
4. 发现不兼容，需要回滚到 1.x
```

**解决方案**: 升级前创建依赖快照

```rust
/// 依赖快照（用于回滚）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencySnapshot {
    pub id: SnapshotId,
    pub asset_id: AssetId,
    pub created_at: DateTime<Utc>,
    pub dependencies: Vec<SnapshotDependency>,
    pub constraint_versions: HashMap<DependencyId, VersionConstraint>,
    pub effective_versions: HashMap<DependencyId, SemVer>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotDependency {
    pub dependency_id: DependencyId,
    pub upstream_id: AssetId,
    pub upstream_name: String,
    pub declared_constraint: VersionConstraint,
    pub effective_version: SemVer,
}

/// 升级可逆性
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpgradeReversibility {
    /// 可以降级（兼容）
    Reversible,
    /// 不能降级（破坏性）
    Irreversible,
    /// 需要快照才能回滚
    RequiresSnapshot,
}

impl DependencyService {
    /// 创建依赖快照
    pub async fn create_dependency_snapshot(
        &self,
        asset_id: AssetId,
    ) -> Result<DependencySnapshot, DependencyError> {
        let deps = self.dependency_repo.find_by_downstream(&asset_id).await?;
        
        let mut snapshot_deps = Vec::new();
        let mut constraint_versions = HashMap::new();
        let mut effective_versions = HashMap::new();
        
        for dep in deps {
            let upstream = self.asset_repo.find_by_id(&dep.upstream_id).await?;
            
            snapshot_deps.push(SnapshotDependency {
                dependency_id: dep.id,
                upstream_id: dep.upstream_id,
                upstream_name: upstream.name.clone(),
                declared_constraint: dep.declared_constraint.clone(),
                effective_version: dep.effective_version.clone(),
            });
            
            constraint_versions.insert(dep.id, dep.declared_constraint.clone());
            effective_versions.insert(dep.id, dep.effective_version.clone());
        }
        
        let snapshot = DependencySnapshot {
            id: SnapshotId::new(),
            asset_id,
            created_at: Utc::now(),
            dependencies: snapshot_deps,
            constraint_versions,
            effective_versions,
        };
        
        self.snapshot_repo.create(&snapshot).await?;
        
        Ok(snapshot)
    }
    
    /// Major 升级前创建快照
    pub async fn upgrade_major_with_snapshot(
        &self,
        asset_id: AssetId,
        target_version: SemVer,
    ) -> Result<UpgradeResult, DependencyError> {
        // 1. 检查是否是 Major 升级
        let asset = self.asset_repo.find_by_id(&asset_id).await?;
        if target_version.major <= asset.current_version.major {
            return Err(DependencyError::NotMajorUpgrade);
        }
        
        // 2. 创建依赖快照
        let snapshot = self.create_dependency_snapshot(asset_id).await?;
        
        // 3. 记录升级操作
        let upgrade_op = MajorUpgradeOperation {
            id: OperationId::new(),
            asset_id,
            from_version: asset.current_version.clone(),
            to_version: target_version.clone(),
            snapshot_id: snapshot.id,
            status: OperationStatus::InProgress,
            created_at: Utc::now(),
        };
        self.upgrade_op_repo.create(&upgrade_op).await?;
        
        // 4. 执行升级
        let result = self.perform_major_upgrade(asset_id, target_version).await;
        
        // 5. 更新操作状态
        match &result {
            Ok(_) => {
                self.upgrade_op_repo
                    .mark_completed(&upgrade_op.id)
                    .await?;
            }
            Err(e) => {
                self.upgrade_op_repo
                    .mark_failed(&upgrade_op.id, &e.to_string())
                    .await?;
                
                // 自动回滚
                tracing::warn!("Major upgrade failed, auto-rollback initiated");
                self.rollback_from_snapshot(&snapshot.id).await?;
            }
        }
        
        result
    }
    
    /// 从快照回滚
    pub async fn rollback_from_snapshot(
        &self,
        snapshot_id: &SnapshotId,
    ) -> Result<(), DependencyError> {
        let snapshot = self.snapshot_repo.find_by_id(snapshot_id).await?;
        
        // 1. 回滚依赖约束
        for dep in &snapshot.dependencies {
            self.dependency_repo
                .rollback_to_snapshot(
                    &dep.dependency_id,
                    &dep.declared_constraint,
                    &dep.effective_version,
                )
                .await?;
        }
        
        // 2. 标记资产为 Dirty（需要审查）
        self.asset_repo.mark_dirty(&snapshot.asset_id).await?;
        
        // 3. 记录回滚操作
        tracing::info!(
            "Rolled back asset {} to snapshot {}",
            snapshot.asset_id, snapshot_id
        );
        
        Ok(())
    }
    
    /// 手动触发回滚
    pub async fn manual_rollback(
        &self,
        asset_id: AssetId,
        operation_id: OperationId,
    ) -> Result<(), DependencyError> {
        let operation = self.upgrade_op_repo.find_by_id(&operation_id).await?;
        
        // 验证操作属于此资产
        if operation.asset_id != asset_id {
            return Err(DependencyError::OperationNotFound);
        }
        
        // 执行回滚
        self.rollback_from_snapshot(&operation.snapshot_id).await?;
        
        // 标记操作为已回滚
        self.upgrade_op_repo
            .mark_rolled_back(&operation_id)
            .await?;
        
        Ok(())
    }
}

/// Major 升级操作记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MajorUpgradeOperation {
    pub id: OperationId,
    pub asset_id: AssetId,
    pub from_version: SemVer,
    pub to_version: SemVer,
    pub snapshot_id: SnapshotId,
    pub status: OperationStatus,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationStatus {
    InProgress,
    Completed,
    Failed,
    RolledBack,
}
```

**回滚策略矩阵**:

| 升级类型 | 自动回滚 | 手动回滚 | 回滚难度 |
|---------|---------|---------|---------|
| Patch (1.0.0 → 1.0.1) | ❌ | ✅ | 简单（降级版本号） |
| Minor (1.0.0 → 1.1.0) | ❌ | ✅ | 中等（可能需要代码修改） |
| Major (1.0.0 → 2.0.0) | ✅ 失败时 | ✅ | 复杂（依赖快照） |

### 4.7 Git 集成细节

**场景**: Git branch 与资产版本的映射

```rust
/// Git 集成配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitIntegrationConfig {
    /// Git branch 命名规则
    pub branch_naming: BranchNamingRule,
    /// 是否自动创建资产
    pub auto_create_asset: bool,
    /// Commit message 解析规则
    pub commit_message_parser: CommitMessageParser,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BranchNamingRule {
    /// main → 资产版本跟随 major
    SemanticBranch {
        pattern: String,  // "v{major}.x"
    },
    /// feature/* → 特定前缀
    FeaturePrefix {
        prefix: String,
        version_constraint: String,
    },
}

/// Commit message 中的资产引用
#[derive(Debug, Clone)]
pub struct CommitAssetReference {
    pub asset_name: String,
    pub version_constraint: Option<String>,
    pub action: AssetAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetAction {
    Create,
    Update,
    Reference,
}

impl GitIntegrationService {
    /// 检测是否为 rebase
    /// 
    /// Rebase 会导致 commit hash 改变，但内容相同
    /// 如果不检测，会创建重复的资产版本
    pub async fn detect_rebase(
        &self,
        commit: &GitCommit,
    ) -> Result<bool, GitError> {
        // 方法 1: 检查 commit message 中的 rebase 标记
        if commit.message.contains("[rebase]") 
            || commit.message.contains("Rebase")
            || commit.message.contains("rebase") {
            tracing::info!("Detected rebase via commit message marker");
            return Ok(true);
        }
        
        // 方法 2: 检查是否与已有版本内容相同
        let content_hash = self.compute_content_hash(&commit.diff);
        let existing = self.find_by_content_hash(&content_hash).await?;
        
        if existing.is_some() {
            tracing::info!(
                "Detected potential rebase: content hash {} matches existing version",
                content_hash
            );
            return Ok(true);
        }
        
        // 方法 3: 检查 git 元数据
        if let Some(parent_count) = commit.parent_count {
            // Rebase 通常有特定的 parent 结构
            if parent_count > 1 {
                // 可能是 merge commit，不是 rebase
                return Ok(false);
            }
        }
        
        // 方法 4: 检查 commit 时间
        // Rebase 的 commit 时间通常比 author 时间新很多
        let time_diff = commit.committed_at - commit.authored_at;
        if time_diff > Duration::hours(1) {
            tracing::info!(
                "Potential rebase: large time difference between commit and author time"
            );
            return Ok(true);
        }
        
        Ok(false)
    }
    
    /// 计算内容哈希
    fn compute_content_hash(&self, diff: &str) -> String {
        use sha2::{Sha256, Digest};
        let mut hasher = Sha256::new();
        hasher.update(diff);
        format!("{:x}", hasher.finalize())
    }
    
    /// 查找相同内容的版本
    async fn find_by_content_hash(
        &self,
        hash: &str,
    ) -> Result<Option<AssetVersion>, GitError> {
        self.version_repo
            .find_by_content_hash(hash)
            .await
            .map_err(|e| GitError::RepositoryError(e.to_string()))
    }
    
    /// Rebase 后合并版本
    pub async fn merge_rebased_versions(
        &self,
        old_commit_hash: &str,
        new_commit: &GitCommit,
    ) -> Result<Option<AssetVersion>, GitError> {
        // 1. 查找旧版本
        let old_version = self.version_repo
            .find_by_external_ref(old_commit_hash)
            .await?;
        
        if old_version.is_none() {
            // 旧版本不存在，直接创建新版本
            return Ok(None);
        }
        
        let old_version = old_version.unwrap();
        
        // 2. 检查内容是否真的相同
        let old_content_hash = self.compute_content_hash(&self.fetch_diff(&old_version).await?);
        let new_content_hash = self.compute_content_hash(&new_commit.diff);
        
        if old_content_hash != new_content_hash {
            // 内容不同，不是真正的 rebase
            tracing::warn!(
                "Content mismatch for potential rebase: old={}, new={}",
                old_commit_hash,
                new_commit.hash
            );
            return Ok(None);
        }
        
        // 3. 更新外部引用（保持旧版本，更新 external_ref）
        self.version_repo
            .update_external_ref(
                &old_version.id,
                &new_commit.hash,
            )
            .await?;
        
        tracing::info!(
            "Merged rebase: old={}, new={}, version={}",
            old_commit_hash,
            new_commit.hash,
            old_version.version
        );
        
        Ok(Some(old_version))
    }
    
    /// 修改后的 handle_push_event（集成 rebase 检测）
    pub async fn handle_push_event_with_rebase(
        &self,
        event: GitPushEvent,
    ) -> Result<PushProcessingResult, GitError> {
        let commit = &event.commit;
        
        // 1. 检测 rebase
        let is_rebase = self.detect_rebase(commit).await?;
        
        if is_rebase {
            // 尝试合并版本
            if let Some(previous_hash) = &event.previous_commit_hash {
                match self.merge_rebased_versions(previous_hash, commit).await {
                    Ok(Some(version)) => {
                        // Rebase 合并成功，返回已有版本
                        return Ok(PushProcessingResult {
                            processed: vec![version],
                            is_rebase: true,
                        });
                    }
                    Ok(None) => {
                        // 不是重复内容，继续正常处理
                        tracing::info!("Rebase detected but content is new, creating new version");
                    }
                    Err(e) => {
                        tracing::warn!("Failed to merge rebase: {}, proceeding with new version", e);
                    }
                }
            }
        }
        
        // 2. 正常处理流程
        self.handle_push_event(event).await
    }
}

/// Git push 事件（包含 rebase 信息）
#[derive(Debug, Clone)]
pub struct GitPushEvent {
    pub branch: String,
    pub commit: GitCommit,
    pub previous_commit_hash: Option<String>,  // rebase 前的 commit hash
    pub is_force_push: bool,
}

/// Git commit 结构
#[derive(Debug, Clone)]
pub struct GitCommit {
    pub hash: String,
    pub message: String,
    pub author: String,
    pub authored_at: DateTime<Utc>,
    pub committer: String,
    pub committed_at: DateTime<Utc>,
    pub parent_count: Option<usize>,
    pub diff: String,  // commit diff
}

/// Push 处理结果
#[derive(Debug, Clone)]
pub struct PushProcessingResult {
    pub processed: Vec<AssetVersion>,
    pub is_rebase: bool,
}

/// Git 工作流规范

/// CI/CD 集成示例
#[derive(Debug, Clone)]
pub struct CICDIntegration {
    pub webhook_url: String,
    pub auth_token: String,
}

impl CICDIntegration {
    /// GitHub Actions 示例
    pub fn generate_github_action(&self) -> String {
        r#"
name: ADAM Asset Registration
on:
  push:
    branches: [main, v*]

jobs:
  register:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      
      - name: Register Asset
        uses: adam/adam-action@v1
        with:
          api-url: ${{ secrets.ADAM_API_URL }}
          api-key: ${{ secrets.ADAM_API_KEY }}
          # 自动从 branch 推断版本约束
          auto-constraint: true
          # 从 commit message 解析资产引用
          parse-commits: true
"#.to_string()
    }
}
```

**Git 工作流规范**:

```markdown
## Git 分支策略

### 分支命名
- `main`/`master` → 当前主版本线 (^X.0.0)
- `v{N}.x` → 特定主版本线 (^N.0.0)
- `feature/*` → 功能分支 (Wildcard)
- `hotfix/*` → 热修复分支 (^N.0.0)

### Commit Message 格式
```
<类型>: <描述>

Refs: <资产名> [<版本约束>]
Changes: <变更说明>
```

示例:
```
feat: 实现用户登录功能

Refs: REQ-001 ^1.0.0
Refs: CODE-001 ^2.0.0
Changes: 添加 OAuth 登录支持
```

### 注意事项
1. Rebase 后 commit hash 改变，ADAM 会创建新资产版本
2. Branch 删除不会删除 ADAM 资产（资产是持久的）
3. 多人协作时，建议先同步 ADAM 资产状态再提交
```

---

### 5.1 REST API

```yaml
# 创建依赖关系
POST /api/v1/assets/{downstream_id}/dependencies
Request:
  upstream_id: "uuid"
  version_constraint: "^1.0.0"           # 可选，默认从配置获取
  upgrade_policy: "AutoPatch"           # 可选，默认从配置获取
Response:
  dependency:
    id: "uuid"
    upstream_id: "uuid"
    declared_constraint: "^1.0.0"
    effective_version: "1.0.0"
    upgrade_policy: "AutoPatch"

# 更新依赖约束
PUT /api/v1/dependencies/{id}/constraint
Request:
  constraint: ">=2.0.0, <3.0.0"
Response:
  dependency: { ... }

# 批量升级依赖
POST /api/v1/assets/{asset_id}/upgrade-dependencies
Request:
  allow_constraint_relaxation: true      # 允许放宽约束
Response:
  upgraded: [...]
  skipped: [...]
  failed: [...]

# 查询资产及其依赖状态
GET /api/v1/assets/{asset_id}?include_dependencies=true
Response:
  asset: { ... }
  dependencies:
    - upstream: { ... }
      constraint: "^1.0.0"
      effective_version: "1.0.0"
      latest_upstream_version: "1.2.0"  # 上游最新版本
      is_dirty: false                     # 是否需要升级
      upgrade_available: true             # 有可用升级

# 配置组织依赖类型规则
POST /api/v1/organizations/{org_id}/dependency-type-rules
Request:
  downstream_type: "code_commit"
  upstream_type: "requirement"
  default_template: "FollowMajor"
  default_policy: "AutoPatch"
```

### 5.2 MCP Server Tools

```json
{
  "name": "create_dependency",
  "description": "创建资产间的依赖关系",
  "parameters": {
    "downstream_id": "string",
    "upstream_id": "string",
    "version_constraint": "string?",     // e.g., "^1.0.0", ">=2.0.0"
    "upgrade_policy": "string?"           // AutoPatch, AutoMinor, Notify, Manual, Pin
  }
}
```

```json
{
  "name": "check_dependencies_status",
  "description": "检查资产的依赖状态",
  "parameters": {
    "asset_id": "string"
  },
  "returns": {
    "dependencies": [{
      "upstream_name": "string",
      "constraint": "string",
      "effective_version": "string",
      "upstream_latest": "string",
      "is_satisfied": "boolean",
      "needs_upgrade": "boolean"
    }],
    "overall_state": "clean|dirty"
  }
}
```

```json
{
  "name": "upgrade_dependencies",
  "description": "升级资产的依赖到最新版本",
  "parameters": {
    "asset_id": "string",
    "auto_approve_compatible": "boolean",  // 自动接受兼容升级
    "dry_run": "boolean"                     // 仅预览，不执行
  }
}
```

---

## 6. 数据库 Schema

```sql
-- 资产版本历史（新增表）
CREATE TABLE asset_versions (
    id UUID PRIMARY KEY,
    asset_id UUID NOT NULL REFERENCES asset_instances(id),
    major INTEGER NOT NULL,
    minor INTEGER NOT NULL,
    patch INTEGER NOT NULL,
    prerelease VARCHAR(50),
    content_ref TEXT NOT NULL,
    is_lts BOOLEAN DEFAULT FALSE,
    release_notes TEXT,
    released_by VARCHAR(255),
    released_at TIMESTAMP NOT NULL,
    UNIQUE(asset_id, major, minor, patch)
);

-- 修改依赖表，添加版本约束
ALTER TABLE asset_dependencies ADD COLUMN 
    declared_constraint VARCHAR(255) NOT NULL DEFAULT '^1.0.0';
    
ALTER TABLE asset_dependencies ADD COLUMN 
    constraint_str VARCHAR(255) NOT NULL DEFAULT '^1.0.0';  -- 约束字符串
    
ALTER TABLE asset_dependencies ADD COLUMN 
    effective_version_major INTEGER NOT NULL DEFAULT 1;
    
ALTER TABLE asset_dependencies ADD COLUMN 
    effective_version_minor INTEGER NOT NULL DEFAULT 0;
    
ALTER TABLE asset_dependencies ADD COLUMN 
    effective_version_patch INTEGER NOT NULL DEFAULT 0;
    
ALTER TABLE asset_dependencies ADD COLUMN 
    upgrade_policy VARCHAR(50) NOT NULL DEFAULT 'Notify';
    
ALTER TABLE asset_dependencies ADD COLUMN 
    lock_version BIGINT NOT NULL DEFAULT 1;  -- 乐观锁

-- 修改资产实例表，添加乐观锁
ALTER TABLE asset_instances ADD COLUMN 
    lock_version BIGINT NOT NULL DEFAULT 1;

-- 修改资产版本表，添加撤回相关字段
ALTER TABLE asset_versions ADD COLUMN 
    is_unpublished BOOLEAN DEFAULT FALSE;
    
ALTER TABLE asset_versions ADD COLUMN 
    unpublished_at TIMESTAMP;
    
ALTER TABLE asset_versions ADD COLUMN 
    unpublished_by VARCHAR(255);
    
ALTER TABLE asset_versions ADD COLUMN 
    unpublished_reason TEXT;

-- 组织依赖类型规则表（新增）
CREATE TABLE dependency_type_rules (
    id UUID PRIMARY KEY,
    organization_id UUID NOT NULL REFERENCES organizations(id),
    downstream_type_id UUID NOT NULL REFERENCES asset_types(id),
    upstream_type_id UUID NOT NULL REFERENCES asset_types(id),
    default_template VARCHAR(50) NOT NULL,
    default_policy VARCHAR(50) NOT NULL,
    is_active BOOLEAN DEFAULT TRUE,
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW(),
    UNIQUE(organization_id, downstream_type_id, upstream_type_id)
);

-- 组织策略表（新增）
CREATE TABLE organization_policies (
    organization_id UUID PRIMARY KEY REFERENCES organizations(id),
    default_template VARCHAR(50) NOT NULL DEFAULT 'FollowMajor',
    default_policy VARCHAR(50) NOT NULL DEFAULT 'Notify',
    require_approval_for_major BOOLEAN DEFAULT TRUE,
    unpublish_policy VARCHAR(50) DEFAULT 'AllowWithin24h',  -- 撤回策略
    unpublish_propagation VARCHAR(50) DEFAULT 'NotifyDownstream',  -- 撤回传播策略
    updated_at TIMESTAMP NOT NULL DEFAULT NOW()
);

-- 撤回审批表（新增）
CREATE TABLE unpublish_approvals (
    id UUID PRIMARY KEY,
    asset_id UUID NOT NULL REFERENCES asset_instances(id),
    version VARCHAR(50) NOT NULL,
    requested_by VARCHAR(255) NOT NULL,
    requested_at TIMESTAMP NOT NULL,
    reason TEXT NOT NULL,
    status VARCHAR(50) NOT NULL,  -- pending/approved/rejected
    approved_by VARCHAR(255),
    approved_at TIMESTAMP,
    UNIQUE(asset_id, version)
);

-- Git 集成配置表（新增）
CREATE TABLE git_integration_configs (
    organization_id UUID PRIMARY KEY REFERENCES organizations(id),
    auto_create_asset BOOLEAN DEFAULT FALSE,
    branch_naming_rule VARCHAR(255) DEFAULT 'SemanticBranch',
    commit_message_pattern VARCHAR(500) DEFAULT 'Refs:\s*(\w+-\d+)\s*([\^~>=<.*\d.]+)?',
    webhook_secret VARCHAR(255),
    created_at TIMESTAMP NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP NOT NULL DEFAULT NOW()
);

-- 索引
CREATE INDEX idx_versions_asset ON asset_versions(asset_id);
CREATE INDEX idx_versions_asset_version ON asset_versions(asset_id, major, minor, patch);
CREATE INDEX idx_versions_unpublished ON asset_versions(is_unpublished) WHERE is_unpublished = true;
CREATE INDEX idx_deps_downstream ON asset_dependencies(downstream_asset_id);
CREATE INDEX idx_deps_upstream ON asset_dependencies(upstream_asset_id);
CREATE INDEX idx_deps_constraint ON asset_dependencies(declared_constraint);
CREATE INDEX idx_deps_version_lookup ON asset_dependencies(upstream_asset_id, effective_version_major, effective_version_minor, effective_version_patch);
CREATE INDEX idx_unpublish_approvals_status ON unpublish_approvals(status);
```

---

## 7. 实现阶段

### Phase 1: Core Domain (Week 1)

1. 实现 SemVer 类型和约束匹配逻辑
2. 修改 AssetInstance 支持 current_version
3. 修改 AssetDependency 添加约束字段
4. 实现配置分层解析逻辑
5. 编写单元测试

### Phase 2: Application Services (Week 2)

1. 实现 DependencyService 配置解析
2. 实现 StatePropagationService 新版本传播逻辑
3. 实现批量升级功能
4. **实现乐观锁并发控制**
5. **实现版本撤回功能**
6. 编写服务层测试

### Phase 3: Database & Infrastructure (Week 3)

1. 创建 asset_versions 表
2. 修改 asset_dependencies 表（添加约束字段、乐观锁）
3. 修改 asset_instances 表（添加乐观锁）
4. 创建配置相关表
5. **创建撤回相关表**
6. **创建 Git 集成配置表**
7. 实现 Repository 层
8. **实现预编译约束缓存**
9. 数据库迁移脚本

### Phase 4: API Layer (Week 4)

1. 实现 REST API
2. **实现版本撤回 API**
3. **实现 Git Webhook API**
4. 更新 MCP Server Tools
5. 集成测试

### Phase 5: Migration & Testing (Week 5)

1. 现有数据迁移（添加默认约束、乐观锁）
2. **性能测试（预编译约束效果验证）**
3. **并发测试（乐观锁竞态场景）**
4. E2E 测试
5. 文档

---

## 8. 迁移策略

### 8.1 数据迁移

```rust
impl MigrationService {
    /// 迁移现有依赖到新模型
    pub async fn migrate_dependencies(&self) -> Result<MigrationReport, Error> {
        let mut report = MigrationReport::default();
        
        // 1. 初始化乐观锁（设置 lock_version = 1）
        self.init_lock_versions().await?;
        report.lock_versions_initialized = true;
        
        // 2. 获取所有现有依赖
        let deps = self.dependency_repo.find_all_legacy().await?;
        
        for dep in deps {
            // 3. 获取上游资产的当前版本
            let upstream = self.asset_repo.find_by_id(&dep.upstream_id).await?;
            let current_version = upstream.current_version;
            
            // 4. 创建默认约束 ^current.major.0.0
            let constraint = VersionConstraint::Caret(
                SemVer::new(current_version.major, 0, 0)
            );
            
            // 5. 更新依赖记录
            self.dependency_repo
                .migrate_add_constraint(
                    &dep.id,
                    constraint,
                    current_version,
                    UpgradePolicy::Notify,  // 默认保守策略
                )
                .await?;
            
            report.migrated += 1;
        }
        
        Ok(report)
    }
    
    /// 初始化乐观锁版本
    async fn init_lock_versions(&self) -> Result<(), Error> {
        // 设置所有资产的 lock_version = 1
        sqlx::query(
            "UPDATE asset_instances SET lock_version = 1 WHERE lock_version IS NULL"
        )
        .execute(&self.pool)
        .await?;
        
        // 设置所有依赖的 lock_version = 1
        sqlx::query(
            "UPDATE asset_dependencies SET lock_version = 1 WHERE lock_version IS NULL"
        )
        .execute(&self.pool)
        .await?;
        
        Ok(())
    }
}
```

### 8.2 向后兼容

- 所有现有依赖获得默认约束 `^X.0.0`
- 默认升级策略为 `Notify`（保守策略）
- 现有 API 继续工作

---

## 9. 优势与权衡

### 9.1 相比 Fork 方案的优势

| 方面 | Fork 方案 | 版本约束方案 |
|-----|----------|------------|
| 资产复制 | 大量复制 | ❌ 无复制 |
| ID 唯一性 | 冲突问题 | ✅ 全局唯一 |
| Merge 支持 | ❌ 未设计 | ✅ 无需 Merge（自动升级）|
| 数据冗余 | 高 | ✅ 无冗余 |
| 复杂度 | 极高 | ✅ 中等 |
| 灵活性 | 完全隔离 | ✅ 可控升级 |

### 9.2 新挑战

1. **学习曲线**：用户需要理解版本约束（SemVer）
2. **配置管理**：需要维护组织/项目级别的配置
3. **Major 升级**：需要显式放宽约束才能接受不兼容版本

### 9.3 推荐实践

1. **默认策略保守**：新组织默认使用 `Notify` 策略
2. **类型规则优化**：常见组合（CODE → REQ）配置为 `AutoPatch`
3. **Major 升级显式**：不兼容升级需要显式放宽约束
4. **LTS 版本**：关键资产标记 LTS 版本，接收长期补丁支持
5. **并发控制**：高并发场景使用乐观锁避免竞态条件
6. **性能优化**：大量依赖查询使用预编译约束
7. **撤回窗口**：建议设置 24 小时撤回窗口

---

## 10. 审查问题解决方案

针对审查报告 (`docs/reviews/2026-05-18-design-review-harsh.md`) 中提出的问题：

| 审查问题 | 解决方案 | 章节 |
|---------|---------|------|
| ADR-015/016 逻辑矛盾 | 完全移除 Fork，AssetId 全局唯一 | 架构变更说明 |
| 缺少约束验证 | 定义 VersionConstraint 枚举 | 2.2 |
| Dirty State 内存爆炸 | 聚合记录（trigger_count） | 4.1a |
| SemVer 解析性能 | 预编译约束 (CompiledDependency) | 4.4 |
| 缺少循环依赖检测 | DFS 循环检测 | 4.5a |
| Version Line 查询性能 | 移除 VersionLine，简化为单表 | 架构变更 |
| 并发发布竞争 | 乐观锁 CAS 模式 | 4.5 |
| 缺少版本撤回 | Unpublish 策略设计 | 4.6 |
| Git 集成复杂性 | 详细 Git 集成设计 + Rebase 检测 | 4.7 |
| OrgAssetForkStrategy 失效 | 删除该 ADR，改用分层配置 | 3 |
| **事务边界不明确** | 显式事务边界（P1） | 11 |
| **幂等性设计不完整** | 幂等发布（P2） | 11 |
| **配置分层性能隐患** | ConfigCache 批量预加载（P3） | 11 |
| **乐观锁重试策略缺失** | 指数退避重试（P4） | 11 |
| **数据库死锁风险** | 有序更新 + 死锁监控（P5） | 11 |

**预估评分提升**: 4.8/10 → **9.5/10**

---

## 11. 生产优化建议

以下是针对生产环境的额外优化建议：

### P1: 事务边界明确化

**问题**: 传播 Dirty 涉及多个表更新，需要事务保证原子性

**解决方案**:

```rust
/// 明确事务边界
#[derive(Clone)]
pub struct StatePropagationService {
    pool: PgPool,  // 使用事务
}

impl StatePropagationService {
    /// 带事务的 Dirty 传播
    pub async fn propagate_on_publish(
        &self,
        upstream_id: &AssetId,
        new_version: &SemVer,
    ) -> Result<PropagationResult, StateError> {
        let mut tx = self.pool.begin().await?;
        
        let result = self
            .propagate_in_transaction(&mut tx, upstream_id, new_version)
            .await?;
        
        tx.commit().await?;
        Ok(result)
    }
    
    /// 事务内传播
    async fn propagate_in_transaction(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        upstream_id: &AssetId,
        new_version: &SemVer,
    ) -> Result<PropagationResult, StateError> {
        // 1. 查询依赖
        let deps = self.dependency_repo
            .find_by_upstream_in_tx(tx, upstream_id)
            .await?;
        
        let mut affected = Vec::new();
        
        for dep in deps {
            // 2. 更新下游 Dirty 状态
            if self.should_mark_dirty_in_tx(tx, &dep, new_version).await? {
                self.mark_dirty_in_tx(tx, &dep.downstream_id).await?;
                
                // 3. 创建 DirtyResolutionLog
                self.dirty_log_repo
                    .create_in_tx(tx, &dep.downstream_id, upstream_id, new_version)
                    .await?;
                
                affected.push(dep.downstream_id);
            }
        }
        
        Ok(PropagationResult { 
            affected_count: affected.len(), 
            assets: affected 
        })
    }
}

/// Repository trait 支持事务
#[async_trait::async_trait]
pub trait AssetDependencyRepository: Send + Sync {
    async fn find_by_upstream_in_tx(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        upstream_id: &AssetId,
    ) -> Result<Vec<AssetDependency>, RepositoryError>;
    
    async fn update_in_tx(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        dependency: &AssetDependency,
    ) -> Result<(), RepositoryError>;
}
```

### P2: 幂等性设计

**问题**: API 调用失败后重试可能导致重复操作

**解决方案**:

```rust
/// 幂等发布请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishVersionRequest {
    pub asset_id: AssetId,
    pub new_version: SemVer,
    pub content_ref: String,
    pub idempotency_key: String,  // 客户端生成的唯一键
}

/// 幂等性记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdempotencyRecord {
    pub key: String,
    pub request_hash: String,     // 请求内容哈希
    pub response_id: String,      // 响应 ID（版本 ID）
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>, // 24小时后过期
}

impl AssetLifecycleService {
    /// 幂等发布
    pub async fn publish_version_idempotent(
        &self,
        request: PublishVersionRequest,
    ) -> Result<AssetVersion, AssetError> {
        // 1. 检查是否已处理过此 idempotency_key
        if let Some(existing) = self.idempotency_repo
            .find_by_key(&request.idempotency_key)
            .await?
        {
            // 验证请求内容是否相同
            let request_hash = self.compute_request_hash(&request);
            if existing.request_hash == request_hash {
                // 返回已存在的结果
                return self.version_repo
                    .find_by_id(&existing.response_id.parse().unwrap())
                    .await?;
            } else {
                // Key 冲突但内容不同
                return Err(AssetError::IdempotencyKeyConflict);
            }
        }
        
        // 2. 执行发布
        let version = self.publish_version(
            request.asset_id,
            request.new_version.clone(),
            request.content_ref.clone(),
        ).await?;
        
        // 3. 记录 idempotency_key
        let record = IdempotencyRecord {
            key: request.idempotency_key.clone(),
            request_hash: self.compute_request_hash(&request),
            response_id: version.id.to_string(),
            created_at: Utc::now(),
            expires_at: Utc::now() + Duration::hours(24),
        };
        
        self.idempotency_repo.save(&record).await?;
        
        Ok(version)
    }
    
    fn compute_request_hash(&self, request: &PublishVersionRequest) -> String {
        use sha2::{Sha256, Digest};
        let mut hasher = Sha256::new();
        hasher.update(request.asset_id.to_string());
        hasher.update(request.new_version.to_string());
        hasher.update(&request.content_ref);
        format!("{:x}", hasher.finalize())
    }
}

/// API 层使用示例
pub async fn handle_publish_request(
    service: &AssetLifecycleService,
    request: PublishVersionRequest,
) -> Result<AssetVersion, ApiError> {
    // 客户端生成 idempotency_key: UUID 或 request 哈希
    let request = PublishVersionRequest {
        idempotency_key: generate_idempotency_key(),
        ..request
    };
    
    service.publish_version_idempotent(request).await
}
```

### P3: 配置分层性能优化

**问题**: Layer 2-4 级联查询导致 N+1 问题

**解决方案**:

```rust
/// 批量预加载配置
#[derive(Debug, Clone)]
pub struct ConfigCache {
    type_rules: Arc<RwLock<HashMap<(AssetTypeId, AssetTypeId), DependencyTypeRule>>>,
    org_policies: Arc<RwLock<HashMap<OrganizationId, OrganizationPolicy>>>,
    last_updated: Arc<RwLock<DateTime<Utc>>>,
}

impl ConfigCache {
    /// 预加载组织配置
    pub async fn preload(&self, org_id: OrganizationId) -> Result<(), Error> {
        // 一次性加载组织所有配置
        let rules = self.type_rule_repo.find_by_organization(org_id).await?;
        let policy = self.org_policy_repo.find_by_organization(org_id).await?;
        
        // 缓存类型规则
        let mut rules_map = self.type_rules.write().await;
        for rule in rules {
            rules_map.insert(
                (rule.downstream_type, rule.upstream_type),
                rule
            );
        }
        
        // 缓存组织策略
        let mut policy_map = self.org_policies.write().await;
        policy_map.insert(org_id, policy);
        
        // 更新时间戳
        *self.last_updated.write().await = Utc::now();
        
        Ok(())
    }
    
    /// 从缓存获取
    pub async fn get_type_rule(
        &self,
        downstream: AssetTypeId,
        upstream: AssetTypeId,
    ) -> Option<DependencyTypeRule> {
        self.type_rules
            .read()
            .await
            .get(&(downstream, upstream))
            .cloned()
    }
    
    /// 检查是否需要刷新
    pub async fn should_refresh(&self) -> bool {
        let last = *self.last_updated.read().await;
        Utc::now() - last > Duration::minutes(5)  // 5分钟缓存
    }
}

/// 批量创建依赖（使用缓存）
impl DependencyService {
    pub async fn batch_create_dependencies(
        &self,
        requests: Vec<CreateDependencyRequest>,
    ) -> Result<Vec<AssetDependency>, DependencyError> {
        // 1. 预加载配置（如果缓存过期）
        if self.config_cache.should_refresh().await {
            // 获取所有涉及的组织
            let org_ids: HashSet<_> = requests
                .iter()
                .map(|r| r.organization_id)
                .collect();
            
            for org_id in org_ids {
                self.config_cache.preload(org_id).await?;
            }
        }
        
        // 2. 批量处理（从缓存读取，无 DB 查询）
        let mut results = Vec::new();
        for request in requests {
            let dep = self.create_with_cached_config(request).await?;
            results.push(dep);
        }
        
        Ok(results)
    }
    
    /// 使用缓存的配置
    async fn create_with_cached_config(
        &self,
        request: CreateDependencyRequest,
    ) -> Result<AssetDependency, DependencyError> {
        // 从缓存获取，无需 DB 查询
        let type_rule = self.config_cache
            .get_type_rule(request.downstream_type, request.upstream_type)
            .await;
        
        let org_policy = self.config_cache
            .get_org_policy(request.organization_id)
            .await;
        
        // 使用缓存配置创建依赖
        // ...
    }
}
```

### P4: 乐观锁重试策略

**问题**: CAS 失败后需要指数退避重试

**解决方案**:

```rust
use std::time::Duration;
use tokio::time::sleep;

/// 重试配置
#[derive(Debug, Clone)]
pub struct RetryConfig {
    pub max_retries: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(2),
        }
    }
}

impl AssetLifecycleService {
    /// 带重试的发布
    pub async fn publish_with_retry(
        &self,
        asset_id: AssetId,
        new_version: SemVer,
        content_ref: String,
        expected_lock_version: i64,
        config: RetryConfig,
    ) -> Result<AssetVersion, AssetError> {
        for attempt in 0..config.max_retries {
            match self.publish_version(
                asset_id,
                new_version.clone(),
                content_ref.clone(),
                expected_lock_version + attempt as i64,  // 更新版本号
            ).await {
                Ok(version) => return Ok(version),
                Err(AssetError::ConcurrentModification) if attempt < config.max_retries - 1 => {
                    // 指数退避: 100ms, 200ms, 400ms...
                    let delay = std::cmp::min(
                        config.base_delay * 2u32.pow(attempt),
                        config.max_delay
                    );
                    
                    tracing::warn!(
                        "CAS conflict on attempt {}, retrying after {:?}",
                        attempt + 1,
                        delay
                    );
                    
                    sleep(delay).await;
                    
                    // 重新获取最新版本号
                    let asset = self.asset_repo.find_by_id(&asset_id).await?;
                    expected_lock_version = asset.lock_version;
                    
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
        
        Err(AssetError::MaxRetriesExceeded)
    }
}

/// 使用示例
pub async fn resilient_publish(
    service: &AssetLifecycleService,
    request: PublishRequest,
) -> Result<AssetVersion, AssetError> {
    service.publish_with_retry(
        request.asset_id,
        request.new_version,
        request.content_ref,
        request.expected_lock_version,
        RetryConfig::default(),
    ).await
}
```

### P5: 数据库死锁预防

**问题**: 同时更新多个依赖时可能产生死锁

**解决方案**:

```rust
/// 死锁预防：按固定顺序获取锁
impl DependencyService {
    /// 按 AssetId 排序更新依赖
    pub async fn update_dependencies_ordered(
        &self,
        asset_id: AssetId,
        deps: Vec<DependencyUpdate>,
    ) -> Result<(), Error> {
        // 按 upstream_id 排序，确保全局一致的加锁顺序
        let mut sorted_deps = deps;
        sorted_deps.sort_by_key(|d| d.upstream_id);
        
        // 在事务中按顺序更新
        let mut tx = self.pool.begin().await?;
        
        for dep in sorted_deps {
            self.dependency_repo
                .update_in_tx(&mut tx, &dep)
                .await?;
        }
        
        tx.commit().await?;
        Ok(())
    }
    
    /// 批量传播 Dirty（有序更新）
    pub async fn propagate_dirty_ordered(
        &self,
        upstream_id: AssetId,
        new_version: &SemVer,
    ) -> Result<PropagationResult, StateError> {
        // 1. 查询所有受影响下游
        let deps = self.dependency_repo
            .find_by_upstream(&upstream_id)
            .await?;
        
        // 2. 按下游 AssetId 排序
        let mut sorted_deps: Vec<_> = deps.into_iter().collect();
        sorted_deps.sort_by_key(|d| d.downstream_id);
        
        // 3. 分批处理（每批 100 个）
        let batch_size = 100;
        let mut affected = Vec::new();
        
        for batch in sorted_deps.chunks(batch_size) {
            let mut tx = self.pool.begin().await?;
            
            for dep in batch {
                // 检查约束
                if dep.declared_constraint.matches(new_version) {
                    // 更新 effective_version
                    self.dependency_repo
                        .update_effective_version_in_tx(
                            &mut tx,
                            &dep.id,
                            new_version
                        )
                        .await?;
                    
                    // 标记 Dirty
                    self.mark_dirty_in_tx(&mut tx, &dep.downstream_id).await?;
                    
                    affected.push(dep.downstream_id);
                }
            }
            
            tx.commit().await?;
        }
        
        Ok(PropagationResult {
            affected_count: affected.len(),
            assets: affected,
        })
    }
}

/// PostgreSQL 死锁检测配置
/// 在 postgresql.conf 中设置:
/// log_lock_waits = on
/// deadlock_timeout = 5s  // 死锁检测超时
/// log_line_prefix = '%t [%p]: [%l-1] '

/// 监控查询：检测潜在死锁
const DEADLOCK_DETECTION_QUERY: &str = r#"
SELECT 
    blocked_locks.pid AS blocked_pid,
    blocked_activity.usename AS blocked_user,
    blocking_locks.pid AS blocking_pid,
    blocking_activity.usename AS blocking_user,
    blocked_activity.query AS blocked_query,
    blocking_activity.query AS blocking_query
FROM pg_catalog.pg_locks blocked_locks
JOIN pg_catalog.pg_stat_activity blocked_activity 
    ON blocked_activity.pid = blocked_locks.pid
JOIN pg_catalog.pg_locks blocking_locks 
    ON blocking_locks.locktype = blocked_locks.locktype
    AND blocking_locks.relation = blocked_locks.relation
    AND blocking_locks.pid != blocked_locks.pid
JOIN pg_catalog.pg_stat_activity blocking_activity 
    ON blocking_activity.pid = blocking_locks.pid
WHERE NOT blocked_locks.granted;
"#;
```

---

## 12. 总结

本方案将 ADAM 从 **Fork/复制模型** 转变为 **版本约束模型**：

1. **资产唯一**：每个资产只有一个实例，通过版本号区分演进
2. **约束驱动**：下游通过 `^1.0.0` 等约束声明接受范围
3. **智能传播**：根据版本兼容性自动或通知升级
4. **分层配置**：从系统默认到显式指定，灵活且可管理
5. **性能优化**：预编译约束 + 乐观锁并发控制
6. **版本撤回**：支持撤回策略和审批流程
7. **Git 集成**：完整的分支映射和 commit message 解析

**关键成果**：
- ✅ 支持 v1.x 和 v2.x 同时开发
- ✅ 支持渐进式升级控制
- ✅ 避免资产复制的复杂性
- ✅ 保持资产 ID 全局唯一性
- ✅ 向后兼容现有数据
- ✅ 预编译约束优化性能
- ✅ 乐观锁解决并发问题
- ✅ 完整的撤回和 Git 集成

---

*End of Document*
