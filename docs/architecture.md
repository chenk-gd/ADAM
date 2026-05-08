# ADAM 系统架构设计文档

**版本**: 1.1  
**日期**: 2026-05-08  
**状态**: 修订稿（已根据审查意见更新）

---

## 架构不变量（Architecture Invariants）

在进行详细设计之前，以下核心约束必须在整个系统范围内保持一致：

| 不变量 | 定义 | 约束位置 |
|--------|------|----------|
| **依赖方向** | `source_id` = 下游/依赖方，`target_id` = 上游/被依赖方 | 数据库、代码、文档 |
| **状态存储** | `current_state` 仅存储枚举标签，Dirty详情存 `dirty_queue` 表，Archived详情存 `asset_instances` 字段 | 数据库 schema |
| **版本策略** | 资产类型可配置版本策略：SemVer / ExternalRef / Composite | AssetType.version_strategy |
| **层级边界** | 持久化依赖边只允许项目内项目级资产之间、或组织内组织级资产之间；项目查询可合并组织级资产但不形成依赖边 | 应用服务层验证、数据库约束 |
| **发布事务** | 发布资产 = 创建版本 + 更新当前版本 + 可选触发下游 Dirty | 事务边界 |

---

## 1. 架构概述

### 1.1 设计目标

ADAM 采用**六边形架构（Hexagonal Architecture）**结合**领域驱动设计（DDD）**原则，实现以下目标：

- **核心业务独立性**：领域逻辑不依赖任何外部框架
- **可测试性**：领域层可独立单元测试
- **可扩展性**：支持新资产类型的插件化扩展
- **高性能**：支持百万级资产实例和复杂 DAG 查询
- **可靠性**：严格的数据一致性保证

### 1.2 架构分层

```
┌─────────────────────────────────────────────────────────────┐
│                      适配器层 (Adapters)                    │
│  ┌────────────┐  ┌────────────┐  ┌────────────┐            │
│  │ REST API   │  │ MCP Server │  │  CLI Tool  │            │
│  │ (axum)     │  │ (mcp-sdk)  │  │ (clap)     │            │
│  └────────────┘  └────────────┘  └────────────┘            │
│  ┌────────────┐  ┌────────────┐  ┌────────────┐            │
│  │ Git Hooks  │  │ CI/CD      │  │ Webhook    │            │
│  └────────────┘  └────────────┘  └────────────┘            │
└─────────────────────────────────────────────────────────────┘
                              │
┌─────────────────────────────────────────────────────────────┐
│                     应用层 (Application)                    │
│  ┌────────────────────────────────────────────────────┐   │
│  │              应用服务 (Application Services)          │   │
│  │  ┌──────────┐ ┌──────────┐ ┌──────────┐             │   │
│  │  │ Asset    │ │ Version  │ │ Impact   │             │   │
│  │  │ Service  │ │ Service  │ │ Analysis │             │   │
│  │  └──────────┘ └──────────┘ └──────────┘             │   │
│  └────────────────────────────────────────────────────┘   │
│  ┌────────────────────────────────────────────────────┐   │
│  │              用例 (Use Cases)                       │   │
│  │  ┌──────────┐ ┌──────────┐ ┌──────────┐             │   │
│  │  │ Publish  │ │ Clean    │ │ Query    │             │   │
│  │  │ Asset    │ │ Asset    │ │ Context  │             │   │
│  │  └──────────┘ └──────────┘ └──────────┘             │   │
│  └────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
                              │
┌─────────────────────────────────────────────────────────────┐
│                      领域层 (Domain)                        │
│  ┌────────────────────────────────────────────────────┐   │
│  │              领域模型 (Domain Models)                 │   │
│  │  ┌──────────┐ ┌──────────┐ ┌──────────┐             │   │
│  │  │ Asset    │ │ Version  │ │Dependency│             │   │
│  │  │ Instance │ │          │ │          │             │   │
│  │  └──────────┘ └──────────┘ └──────────┘             │   │
│  │  ┌──────────┐ ┌──────────┐ ┌──────────┐             │   │
│  │  │ Asset    │ │ Dirty    │ │ Virtual  │             │   │
│  │  │ Type     │ │ State    │ │ Instance │             │   │
│  │  └──────────┘ └──────────┘ └──────────┘             │   │
│  └────────────────────────────────────────────────────┘   │
│  ┌────────────────────────────────────────────────────┐   │
│  │              领域服务 (Domain Services)             │   │
│  │  ┌──────────┐ ┌──────────┐ ┌──────────┐             │   │
│  │  │ DAG      │ │ State    │ │ Version  │             │   │
│  │  │ Validator│ │ Propagator│ │ Suggester│             │   │
│  │  └──────────┘ └──────────┘ └──────────┘             │   │
│  └────────────────────────────────────────────────────┘   │
│  ┌────────────────────────────────────────────────────┐   │
│  │              领域事件 (Domain Events)               │   │
│  │  AssetPublished | AssetCleaned | StateChanged        │   │
│  └────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
                              │
┌─────────────────────────────────────────────────────────────┐
│                    基础设施层 (Infrastructure)                │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────┐  │
│  │ Database │  │ Message  │  │ External │  │ Content  │  │
│  │ (PostgreSQL)│ │ Queue    │  │ Systems  │  │ Storage  │  │
│  └──────────┘  └──────────┘  └──────────┘  └──────────┘  │
└─────────────────────────────────────────────────────────────┘
```

---

## 2. 模块结构

### 2.1 目录布局

```
adam/
├── Cargo.toml
├── src/
│   ├── main.rs                 # 程序入口
│   ├── lib.rs                  # 库导出
│   ├── config.rs               # 配置管理
│   ├── error.rs                # 全局错误定义
│   ├── domain/                 # 领域层
│   │   ├── mod.rs
│   │   ├── asset/              # 资产模块
│   │   │   ├── mod.rs
│   │   │   ├── entity.rs       # AssetInstance, AssetType
│   │   │   ├── value_object.rs # AssetId, Version, etc.
│   │   │   ├── repository.rs   # 仓库接口
│   │   │   └── events.rs       # 领域事件
│   │   ├── dependency/         # 依赖模块
│   │   │   ├── mod.rs
│   │   │   ├── entity.rs       # AssetDependency
│   │   │   ├── dag.rs          # DAG 验证算法
│   │   │   ├── graph.rs        # 依赖图操作
│   │   │   └── repository.rs
│   │   ├── version/              # 版本模块
│   │   │   ├── mod.rs
│   │   │   ├── entity.rs       # AssetVersion
│   │   │   ├── semver.rs       # SemVer 处理
│   │   │   └── repository.rs
│   │   ├── state/                # 状态模块
│   │   │   ├── mod.rs
│   │   │   ├── machine.rs      # 状态机
│   │   │   ├── propagation.rs  # 状态传播
│   │   │   └── log.rs          # DirtyResolutionLog
│   │   └── virtual/              # 虚拟实例模块
│   │       ├── mod.rs
│   │       ├── entity.rs       # VirtualInstance
│   │       └── service.rs
│   ├── application/              # 应用层
│   │   ├── mod.rs
│   │   ├── asset_service.rs    # 资产应用服务
│   │   ├── version_service.rs  # 版本应用服务
│   │   ├── impact_service.rs   # 影响分析服务
│   │   ├── virtual_service.rs  # 虚拟实例服务
│   │   └── dto.rs              # DTO 定义
│   ├── ports/                    # 端口（接口定义）
│   │   ├── mod.rs
│   │   ├── http/               # REST API 端口
│   │   │   ├── mod.rs
│   │   │   ├── routes.rs
│   │   │   ├── handlers/
│   │   │   └── middleware/
│   │   └── mcp/                # MCP 协议端口
│   │       ├── mod.rs
│   │       ├── server.rs
│   │       ├── tools.rs
│   │       └── resources.rs
│   ├── infrastructure/           # 基础设施层
│   │   ├── mod.rs
│   │   ├── persistence/        # 持久化
│   │   │   ├── mod.rs
│   │   │   ├── postgres.rs
│   │   │   ├── repositories/
│   │   │   └── migrations/
│   │   ├── messaging/          # 消息队列
│   │   │   ├── mod.rs
│   │   │   └── redis.rs
│   │   ├── external/           # 外部系统集成
│   │   │   ├── mod.rs
│   │   │   ├── git.rs
│   │   │   ├── wiki.rs
│   │   │   └── ci.rs
│   │   └── content/            # 内容代理
│   │       ├── mod.rs
│   │       └── proxy.rs
│   └── shared/                   # 共享组件
│       ├── mod.rs
│       ├── types.rs
│       └── utils/
├── tests/                        # 集成测试
├── benches/                      # 性能测试
└── docs/
    ├── architecture.md           # 本文件
    └── spec.md                   # 需求规格
```

---

## 3. 核心领域模型

### 3.1 实体定义

#### AssetId - 资产标识符（Newtype 模式）

```rust
use uuid::Uuid;

/// 资产实例唯一标识符
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AssetId(pub Uuid);

impl AssetId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Display for AssetId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// 资产类型标识符
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AssetTypeId(pub Uuid);

/// 组织标识符
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OrganizationId(pub Uuid);

/// 项目标识符
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProjectId(pub Uuid);
```

#### AssetState - 资产状态（简化枚举，详情存储分离）

**设计说明**：AssetState 仅存储状态标签，Dirty 状态的详细信息（pending_upstream、影响等级等）存储在单独的 `dirty_queue` 表中，Archived 的归档信息存储在 asset_instances 的 `archived_at` 和 `archived_reason` 字段。

```rust
/// 资产生命周期状态（仅标签，不包含详情）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(rename_all = "lowercase")]
pub enum AssetState {
    /// 清洁状态：依赖已同步到最新版本
    Clean,
    /// 脏状态：上游有新版本待处理（详情见 dirty_queue 表）
    Dirty,
    /// 归档状态：已停止维护（详情见 archived_at, archived_reason 字段）
    Archived,
}

impl AssetState {
    /// 是否为 Dirty 状态
    pub fn is_dirty(&self) -> bool {
        matches!(self, AssetState::Dirty)
    }
    
    /// 是否为 Archived 状态
    pub fn is_archived(&self) -> bool {
        matches!(self, AssetState::Archived)
    }
    
    /// 是否可以转换到目标状态（不返回错误，仅返回布尔）
    pub fn can_transition_to(&self, new_state: AssetState) -> bool {
        self.validate_transition(new_state).is_ok()
    }

    /// 是否可以发布新版本
    pub fn can_publish(&self) -> bool {
        matches!(self, AssetState::Clean | AssetState::Dirty)
    }

    /// 是否可以建立新依赖
    pub fn can_depend(&self) -> bool {
        matches!(self, AssetState::Clean | AssetState::Dirty)
    }

    /// 状态转换验证
    pub fn validate_transition(&self, new_state: AssetState) -> Result<(), StateError> {
        match (self, new_state) {
            // Archived 是终态，不能接收 Dirty，也不能恢复发布
            (AssetState::Archived, _) => Err(StateError::ArchivedIsTerminal),
            // Clean/Dirty 可以变为 Dirty（通过上游发布）
            (AssetState::Clean, AssetState::Dirty) | (AssetState::Dirty, AssetState::Dirty) => Ok(()),
            // Dirty 可以变为 Clean（通过发布或手工 Clean）
            (AssetState::Dirty, AssetState::Clean) => Ok(()),
            // Clean 保持 Clean（通过发布自身）
            (AssetState::Clean, AssetState::Clean) => Ok(()),
            // Clean/Dirty 可以归档
            (AssetState::Clean, AssetState::Archived) | (AssetState::Dirty, AssetState::Archived) => Ok(()),
            // Clean -> Clean 是合法的（重新发布）
            _ => Err(StateError::InvalidStateTransition {
                from: format!("{:?}", self),
                to: format!("{:?}", new_state),
            }),
        }
    }
}

/// 影响等级
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
pub enum ImpactLevel {
    Low = 1,
    Medium = 2,
    High = 3,
    Critical = 4,
}

/// Dirty 队列条目（独立表存储）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirtyQueueEntry {
    pub id: Uuid,
    /// 受影响的资产（下游）
    pub asset_id: AssetId,
    /// 导致 Dirty 的上游资产
    pub upstream_asset_id: AssetId,
    /// 上游新版本
    pub upstream_version: VersionIdentifier,
    /// 上游资产旧版本（当前有效基线）
    pub upstream_old_version: VersionIdentifier,
    /// 影响等级
    pub impact_level: ImpactLevel,
    /// Dirty 开始时间
    pub since: DateTime<Utc>,
    /// 是否已处理（手工 Clean 或重新发布后）
    pub resolved: bool,
    /// 处理时间
    pub resolved_at: Option<DateTime<Utc>>,
}

impl DirtyQueueEntry {
    /// 计算优先级分数
    /// score = impact_level_weight + min(hours_waiting, 72) * age_weight + asset_type_weight
    pub fn priority_score(&self, asset_type_weight: i32) -> i32 {
        let impact_weight = self.impact_level as i32 * 100;
        let hours_waiting = (Utc::now() - self.since).num_hours();
        let age_weight = hours_waiting.min(72) * 10;
        impact_weight + age_weight as i32 + asset_type_weight
    }
}
```

#### AssetLevel - 资产层级（含组织边界验证）

```rust
/// 资产层级：项目级或组织级
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(rename_all = "lowercase")]
pub enum AssetLevel {
    /// 项目级资产
    Project,
    /// 组织级资产
    Organization,
}

/// 依赖边界验证上下文
pub struct DependencyBoundaryContext {
    pub source_level: AssetLevel,
    pub source_project_id: Option<ProjectId>,
    pub source_org_id: OrganizationId,
    pub target_level: AssetLevel,
    pub target_project_id: Option<ProjectId>,
    pub target_org_id: OrganizationId,
}

impl DependencyBoundaryContext {
    /// 验证依赖关系是否违反组织/项目边界
    /// 
    /// 规则（BR-008）：
    /// 1. 依赖关系不得跨组织建立
    /// 2. 项目级资产只能依赖同一项目内的项目级资产
    /// 3. 组织级资产只能依赖同组织内的组织级资产
    /// 4. 项目级资产不能持久化依赖组织级资产；组织级资产通过查询上下文合并
    pub fn validate(&self) -> Result<(), DependencyError> {
        // 规则1：不得跨组织
        if self.source_org_id != self.target_org_id {
            return Err(DependencyError::CrossOrganizationNotAllowed);
        }

        match (self.source_level, self.target_level) {
            // 项目级 -> 项目级：必须同项目
            (AssetLevel::Project, AssetLevel::Project) => {
                if self.source_project_id != self.target_project_id {
                    return Err(DependencyError::CrossProjectNotAllowed);
                }
                Ok(())
            }
            // 项目级 -> 组织级：禁止；组织级资产仅作为查询上下文合并
            (AssetLevel::Project, AssetLevel::Organization) => {
                Err(DependencyError::ProjectCannotDependOnOrganization)
            }
            // 组织级 -> 组织级：允许（同组织已验证）
            (AssetLevel::Organization, AssetLevel::Organization) => Ok(()),
            // 组织级 -> 项目级：禁止
            (AssetLevel::Organization, AssetLevel::Project) => {
                Err(DependencyError::OrganizationCannotDependOnProject)
            }
        }
    }
}

#### VersionIdentifier - 版本标识符（支持多种策略）

**设计说明**：不同资产类型使用不同的版本标识策略。代码提交使用 commit SHA 或 tag，文档使用 SemVer，流水线运行使用 run ID。

```rust
/// 版本标识策略
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(rename_all = "lowercase")]
pub enum VersionStrategy {
    /// 语义化版本（SemVer）- 适用于文档、设计等
    SemVer,
    /// 外部引用版本 - 直接使用外部系统的标识（commit SHA、tag、run ID等）
    ExternalRef,
    /// 组合版本 - 使用外部标识 + 序列号
    Composite,
}

/// 版本标识符（统一包装）
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct VersionIdentifier(pub String);

impl VersionIdentifier {
    /// 从 SemVer 创建
    pub fn from_semver(version: &SemVersion) -> Self {
        Self(version.to_string())
    }

    /// 从外部引用创建（commit SHA、tag、run ID等）
    pub fn from_external_ref(external_ref: &str) -> Self {
        Self(external_ref.to_string())
    }

    /// 从组合格式创建（例如：tag.001）
    pub fn from_composite(base: &str, sequence: u32) -> Self {
        Self(format!("{}.{:03}", base, sequence))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for VersionIdentifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// 语义化版本（用于需要版本比较和建议的场景）
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SemVersion(semver::Version);

impl SemVersion {
    pub fn parse(s: &str) -> Result<Self, semver::Error> {
        Ok(Self(semver::Version::parse(s)?))
    }

    /// 建议下一个版本号
    pub fn suggest_next(&self, change_type: ChangeType) -> Self {
        let v = &self.0;
        match change_type {
            ChangeType::Major => Self(semver::Version::new(v.major + 1, 0, 0)),
            ChangeType::Minor => Self(semver::Version::new(v.major, v.minor + 1, 0)),
            ChangeType::Patch => Self(semver::Version::new(v.major, v.minor, v.patch + 1)),
        }
    }
}

impl Display for SemVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeType {
    Major, // 破坏性变更
    Minor, // 新增功能
    Patch, // 问题修复
}

#### AssetType - 资产类型（含版本策略）

```rust
use serde_json::Value as JsonValue;

/// 资产类型定义
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetType {
    pub id: AssetTypeId,
    pub name: String,
    pub display_name: String,
    pub description: Option<String>,
    /// 所属组织
    pub organization_id: OrganizationId,
    /// 元数据 JSON Schema
    pub metadata_schema: JsonSchema,
    /// 版本标识策略
    pub version_strategy: VersionStrategy,
    /// 版本保留策略
    pub retention_policy: RetentionPolicy,
    pub icon: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// JSON Schema 包装类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonSchema(pub JsonValue);

/// 保留策略
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionPolicy {
    /// 按数量保留
    pub max_count: Option<u32>,
    /// 按天数保留
    pub max_days: Option<u32>,
    /// 永久保留
    pub permanent: bool,
    /// 不同状态的保留策略
    pub by_status: HashMap<String, RetentionRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionRule {
    pub count: Option<u32>,
    pub days: Option<u32>,
}
```

#### AssetInstance - 资产实例（含组织ID、幂等键、归档字段）

```rust
/// 资产实例
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetInstance {
    pub id: AssetId,
    pub type_id: AssetTypeId,
    pub name: String,
    /// 外部系统引用地址
    pub external_ref: String,
    /// 来源系统
    pub source: AssetSource,
    /// 资产层级
    pub level: AssetLevel,
    /// 所属组织
    pub organization_id: OrganizationId,
    /// 所属项目ID（项目级资产必填）
    pub project_id: Option<ProjectId>,
    /// 当前版本号（根据资产类型的 version_strategy）
    pub current_version: VersionIdentifier,
    /// 当前状态（Clean/Dirty/Archived）
    pub current_state: AssetState,
    /// 归档时间（Archived 状态时有效）
    pub archived_at: Option<DateTime<Utc>>,
    /// 归档原因（Archived 状态时有效）
    pub archived_reason: Option<String>,
    /// 元数据（按类型 schema 存储）
    pub metadata: JsonValue,
    /// 最新版本发布人
    pub publisher: String,
    /// 责任人列表
    pub assignees: Vec<String>,
    /// 幂等键（用于自动化注册防重）
    /// 格式：{source}:{organization_id}:{project_id?}:{external_ref}
    pub idempotency_key: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(rename_all = "lowercase")]
pub enum AssetSource {
    Git,
    Wiki,
    Jira,
    AzureDevOps,
    GitLab,
    GitHub,
    Custom,
}

impl AssetInstance {
    /// 创建新的资产实例（首次发布）
    pub fn new(
        type_id: AssetTypeId,
        name: String,
        external_ref: String,
        source: AssetSource,
        level: AssetLevel,
        organization_id: OrganizationId,
        project_id: Option<ProjectId>,
        initial_version: VersionIdentifier,
        metadata: JsonValue,
        publisher: String,
        assignees: Vec<String>,
    ) -> Result<Self, ValidationError> {
        // 验证层级与项目ID的一致性
        match (level, project_id) {
            (AssetLevel::Project, None) => {
                return Err(ValidationError::ProjectIdRequired)
            }
            (AssetLevel::Organization, Some(_)) => {
                return Err(ValidationError::OrganizationLevelShouldNotHaveProject)
            }
            _ => {}
        }

        // 生成幂等键
        let idempotency_key = Self::generate_idempotency_key(
            &source,
            &organization_id,
            project_id.as_ref(),
            &external_ref,
        );

        Ok(Self {
            id: AssetId::new(),
            type_id,
            name,
            external_ref,
            source,
            level,
            organization_id,
            project_id,
            current_version: initial_version,
            current_state: AssetState::Clean,
            archived_at: None,
            archived_reason: None,
            metadata,
            publisher,
            assignees,
            idempotency_key,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
    }

    /// 生成幂等键
    fn generate_idempotency_key(
        source: &AssetSource,
        org_id: &OrganizationId,
        project_id: Option<&ProjectId>,
        external_ref: &str,
    ) -> String {
        match project_id {
            Some(pid) => format!("{}:{}:{}:{}", 
                format!("{:?}", source).to_lowercase(),
                org_id.0,
                pid.0,
                external_ref
            ),
            None => format!("{}:{}::{}", 
                format!("{:?}", source).to_lowercase(),
                org_id.0,
                external_ref
            ),
        }
    }

    /// 归档资产
    pub fn archive(&mut self, reason: String) -> Result<(), StateError> {
        if matches!(self.current_state, AssetState::Archived) {
            return Err(StateError::AlreadyArchived);
        }
        self.current_state = AssetState::Archived;
        self.archived_at = Some(Utc::now());
        self.archived_reason = Some(reason);
        self.updated_at = Utc::now();
        Ok(())
    }

    /// 获取完整的上下文查询ID
    pub fn context_id(&self) -> String {
        format!("{}:{}", self.source, self.external_ref)
    }
}
```

#### AssetDependency - 实例依赖关系

**依赖方向定义**：
- `source_id` = 依赖方（下游）- 依赖上游的资产
- `target_id` = 被依赖方（上游）- 被依赖的资产
- 边方向：下游(source) -> 上游(target)

**查询方向说明**：
- 查询**上游依赖**（资产依赖谁）：`WHERE source_id = 当前资产ID`
- 查询**下游依赖**（谁依赖资产）：`WHERE target_id = 当前资产ID`

```rust
/// 依赖关系更新原因
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(rename_all = "lowercase")]
pub enum EffectiveUpdateReason {
    /// 通过发布更新
    Publish,
    /// 通过手工审查确认
    ManualClean,
}

/// 资产实例间依赖关系
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetDependency {
    pub id: Uuid,
    /// 依赖方资产ID（下游 - 依赖上游的资产）
    /// 示例：代码提交(source) -> 需求(target)
    pub source_id: AssetId,
    /// 被依赖方资产ID（上游 - 被依赖的资产）
    /// 示例：需求(target) <- 代码提交(source)
    pub target_id: AssetId,
    /// 关系类型
    pub relationship: DependencyRelation,
    /// 发布时声明依赖的上游版本（历史追溯）
    pub declared_version: VersionIdentifier,
    /// 当前有效依赖基线版本（上下文查询和 Dirty 判断）
    pub effective_version: VersionIdentifier,
    /// 当前有效依赖基线最近更新人
    pub effective_updated_by: String,
    /// 当前有效依赖基线最近更新时间
    pub effective_updated_at: DateTime<Utc>,
    /// 当前有效依赖基线更新原因
    pub effective_reason: EffectiveUpdateReason,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(rename_all = "lowercase")]
pub enum DependencyRelation {
    DependsOn,
    References,
}

impl AssetDependency {
    /// 更新当前有效依赖基线（手工审查确认）
    pub fn update_effective_version(
        &mut self,
        new_version: VersionIdentifier,
        updated_by: String,
        reason: EffectiveUpdateReason,
    ) {
        self.effective_version = new_version;
        self.effective_updated_by = updated_by;
        self.effective_updated_at = Utc::now();
        self.effective_reason = reason;
    }
}
```

#### AssetVersion - 版本发布记录

```rust
/// 版本发布记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetVersion {
    pub id: Uuid,
    pub instance_id: AssetId,
    pub version_number: VersionIdentifier,
    /// 版本元数据快照
    pub metadata: JsonValue,
    /// 该版本依赖的上游资产及版本
    pub dependencies: Vec<DependencySnapshot>,
    /// 发布说明
    pub release_notes: Option<String>,
    /// 平台建议的版本类型（仅 SemVer 策略有效）
    pub suggested_type: Option<ChangeType>,
    /// 发布人
    pub released_by: String,
    /// 发布时间
    pub released_at: DateTime<Utc>,
}

/// 依赖快照
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencySnapshot {
    pub asset_id: AssetId,
    pub version: VersionIdentifier,
    pub relation: DependencyRelation,
}

impl AssetVersion {
    /// 创建新版本发布记录
    pub fn new(
        instance_id: AssetId,
        version_number: VersionIdentifier,
        metadata: JsonValue,
        dependencies: Vec<DependencySnapshot>,
        release_notes: Option<String>,
        suggested_type: Option<ChangeType>,
        released_by: String,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            instance_id,
            version_number,
            metadata,
            dependencies,
            release_notes,
            suggested_type,
            released_by,
            released_at: Utc::now(),
        }
    }
}
```

#### DirtyResolutionLog - Dirty 处理日志

```rust
/// 审查结论
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(rename_all = "lowercase")]
pub enum ReviewResult {
    /// 确认无影响
    NoImpact,
    /// 已更新
    Updated,
}

/// Dirty 处理日志
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirtyResolutionLog {
    pub id: Uuid,
    /// 被处理的资产
    pub asset_id: AssetId,
    /// 被处理的资产当前版本
    pub asset_version: VersionIdentifier,
    /// 导致 Dirty 的上游资产
    pub upstream_asset_id: AssetId,
    /// 处理前当前有效依赖基线版本
    pub from_version: VersionIdentifier,
    /// 审查确认后的上游版本
    pub to_version: VersionIdentifier,
    /// 处理动作
    pub action: ResolutionAction,
    /// 审查结论
    pub review_result: ReviewResult,
    /// 审查说明
    pub comment: Option<String>,
    /// 审查人
    pub reviewed_by: String,
    /// 审查时间
    pub reviewed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(rename_all = "lowercase")]
pub enum ResolutionAction {
    ManualClean,
    Republish,
}
```

#### VirtualInstance - 虚拟查询上下文

```rust
/// 虚拟资产实例（临时查询上下文）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VirtualInstance {
    pub id: Uuid,
    /// 目标资产类型
    pub target_type_id: AssetTypeId,
    /// 锚点资产ID列表
    pub anchor_ids: Vec<AssetId>,
    /// 关联的AI会话/请求ID
    pub session_id: String,
    /// 创建者主体ID
    pub created_by: String,
    /// 所属组织
    pub organization_id: OrganizationId,
    /// 所属项目ID
    pub project_id: ProjectId,
    pub created_at: DateTime<Utc>,
    /// 过期时间（短期，如5分钟）
    pub expires_at: DateTime<Utc>,
}

impl VirtualInstance {
    /// 创建虚拟实例
    pub fn new(
        target_type_id: AssetTypeId,
        anchor_ids: Vec<AssetId>,
        session_id: String,
        created_by: String,
        organization_id: OrganizationId,
        project_id: ProjectId,
        ttl_minutes: i64,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            target_type_id,
            anchor_ids,
            session_id,
            created_by,
            organization_id,
            project_id,
            created_at: now,
            expires_at: now + Duration::minutes(ttl_minutes),
        }
    }

    /// 检查是否已过期
    pub fn is_expired(&self) -> bool {
        Utc::now() > self.expires_at
    }

    /// 获取应依赖的完整上游资产上下文
    pub async fn get_context(
        &self,
        dependency_service: &dyn DependencyService,
    ) -> Result<VirtualContext, DomainError> {
        dependency_service.resolve_virtual_context(self).await
    }
}

/// 虚拟实例依赖上下文
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VirtualContext {
    pub target_type: AssetTypeId,
    pub created_by: String,
    pub organization_id: OrganizationId,
    pub project_id: ProjectId,
    pub dependencies: Vec<ContextDependency>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextDependency {
    pub asset_type: AssetTypeId,
    pub asset_type_name: String,
    pub instance_id: AssetId,
    pub instance_name: String,
    pub version: VersionIdentifier,
    pub external_ref: String,
}
```

---

## 4. 领域服务

### 4.1 DAG 验证服务

**依赖方向定义**：
- `source_id` = 下游（依赖方）- 依赖上游的资产
- `target_id` = 上游（被依赖方）- 被依赖的资产  
- 边方向：下游 -> 上游（即：下游依赖于上游）

**查询方向说明**：
- **查询上游依赖**（我依赖谁）：`WHERE source_id = 当前资产ID`
- **查询下游依赖**（谁依赖我）：`WHERE target_id = 当前资产ID`

```rust
use petgraph::graph::DiGraph;
use petgraph::algo::{is_cyclic_directed, toposort};

/// DAG 验证错误
#[derive(Debug, Error)]
pub enum DagError {
    #[error("cycle detected: {0}")]
    CycleDetected(String),
    #[error("node not found: {0}")]
    NodeNotFound(AssetId),
}

/// DAG 验证服务
pub struct DagValidator;

impl DagValidator {
    /// 验证添加新依赖是否会形成环
    /// 
    /// 参数说明：
    /// - existing_edges: Vec<(source_id, target_id)> - 现有依赖边
    /// - new_edge: (source_id, target_id) - 要添加的新依赖
    /// 
    /// 注意：source_id 是下游（依赖方），target_id 是上游（被依赖方）
    pub fn validate_no_cycle(
        existing_edges: &[(AssetId, AssetId)],
        new_edge: (AssetId, AssetId),
    ) -> Result<(), DagError> {
        let mut graph = DiGraph::<AssetId, ()>::new();
        let mut node_indices: HashMap<AssetId, NodeIndex> = HashMap::new();

        // 添加所有节点
        for (source, target) in existing_edges.iter().chain(std::iter::once(&new_edge)) {
            if !node_indices.contains_key(source) {
                node_indices.insert(*source, graph.add_node(*source));
            }
            if !node_indices.contains_key(target) {
                node_indices.insert(*target, graph.add_node(*target));
            }
        }

        // 添加现有边
        for (source, target) in existing_edges {
            let source_idx = node_indices.get(source).copied().unwrap();
            let target_idx = node_indices.get(target).copied().unwrap();
            graph.add_edge(source_idx, target_idx, ());
        }

        // 添加新边
        let source_idx = node_indices.get(&new_edge.0).copied()
            .map_err(|_| DagError::NodeNotFound(new_edge.0))?;
        let target_idx = node_indices.get(&new_edge.1).copied()
            .map_err(|_| DagError::NodeNotFound(new_edge.1))?;
        graph.add_edge(source_idx, target_idx, ());

        // 检查是否有环
        if is_cyclic_directed(&graph) {
            return Err(DagError::CycleDetected(format!(
                "adding dependency {} -> {} would create a cycle",
                new_edge.0, new_edge.1
            )));
        }

        Ok(())
    }

    /// 拓扑排序获取执行顺序
    pub fn topological_sort(nodes: &[AssetId], edges: &[(AssetId, AssetId)]) -> Result<Vec<AssetId>, DagError> {
        let mut graph = DiGraph::<AssetId, ()>::new();
        let mut node_indices: HashMap<AssetId, NodeIndex> = HashMap::new();

        for node in nodes {
            node_indices.insert(*node, graph.add_node(*node));
        }

        for (source, target) in edges {
            if let (Some(&s), Some(&t)) = (node_indices.get(source), node_indices.get(target)) {
                graph.add_edge(s, t, ());
            }
        }

        let sorted = toposort(&graph, None)
            .map_err(|_| DagError::CycleDetected("graph contains cycle".to_string()))?;

        Ok(sorted.iter().map(|idx| graph[*idx]).collect())
    }
}

/// 依赖图查询服务（数据库查询示例）
pub struct DependencyGraphQuery;

impl DependencyGraphQuery {
    /// 查询直接上游依赖（我依赖谁）
    /// SQL: SELECT target_id FROM asset_dependencies WHERE source_id = $1
    pub fn upstream_query() -> &'static str {
        r#"
            SELECT 
                d.target_id as asset_id,
                d.effective_version as version,
                ai.name as asset_name,
                ai.current_state as state
            FROM asset_dependencies d
            JOIN asset_instances ai ON d.target_id = ai.id
            WHERE d.source_id = $1
        "#
    }

    /// 查询直接下游依赖（谁依赖我）
    /// SQL: SELECT source_id FROM asset_dependencies WHERE target_id = $1
    pub fn downstream_query() -> &'static str {
        r#"
            SELECT 
                d.source_id as asset_id,
                d.effective_version as version,
                ai.name as asset_name,
                ai.current_state as state
            FROM asset_dependencies d
            JOIN asset_instances ai ON d.source_id = ai.id
            WHERE d.target_id = $1
        "#
    }

    /// 递归查询上游依赖（间接依赖）
    /// 使用 PostgreSQL 递归 CTE
    pub fn recursive_upstream_query() -> &'static str {
        r#"
            WITH RECURSIVE upstream_tree AS (
                -- 起始节点：直接上游
                SELECT 
                    target_id as asset_id,
                    effective_version as version,
                    1 as depth,
                    ARRAY[target_id] as path
                FROM asset_dependencies
                WHERE source_id = $1
                
                UNION ALL
                
                -- 递归：继续查找上游
                SELECT 
                    d.target_id,
                    d.effective_version,
                    ut.depth + 1,
                    ut.path || d.target_id
                FROM asset_dependencies d
                JOIN upstream_tree ut ON d.source_id = ut.asset_id
                WHERE ut.depth < $2  -- 限制深度防止无限递归
                    AND NOT d.target_id = ANY(ut.path)  -- 避免循环
            )
            SELECT DISTINCT 
                ut.asset_id,
                ai.name,
                ai.current_state,
                ut.depth
            FROM upstream_tree ut
            JOIN asset_instances ai ON ut.asset_id = ai.id
            ORDER BY ut.depth
        "#
    }

    /// 递归查询下游依赖（间接依赖，用于影响分析）
    pub fn recursive_downstream_query() -> &'static str {
        r#"
            WITH RECURSIVE downstream_tree AS (
                -- 起始节点：直接下游
                SELECT 
                    source_id as asset_id,
                    effective_version as version,
                    1 as depth,
                    ARRAY[source_id] as path
                FROM asset_dependencies
                WHERE target_id = $1
                
                UNION ALL
                
                -- 递归：继续查找下游
                SELECT 
                    d.source_id,
                    d.effective_version,
                    dt.depth + 1,
                    dt.path || d.source_id
                FROM asset_dependencies d
                JOIN downstream_tree dt ON d.target_id = dt.asset_id
                WHERE dt.depth < $2
                    AND NOT d.source_id = ANY(dt.path)
            )
            SELECT DISTINCT 
                dt.asset_id,
                ai.name,
                ai.current_state,
                ai.level,
                dt.depth
            FROM downstream_tree dt
            JOIN asset_instances ai ON dt.asset_id = ai.id
            ORDER BY dt.depth
        "#
    }
}
```

### 4.2 状态传播服务

```rust
/// 状态传播服务
pub struct StatePropagator {
    event_publisher: Box<dyn EventPublisher>,
}

impl StatePropagator {
    pub fn new(event_publisher: Box<dyn EventPublisher>) -> Self {
        Self { event_publisher }
    }

    /// 处理资产发布事件
    /// 核心规则：只有发布新版本才会触发直接下游 Dirty
    pub async fn on_asset_published(
        &self,
        asset_id: AssetId,
        new_version: VersionIdentifier,
        repo: &dyn AssetRepository,
        dirty_queue_repo: &dyn DirtyQueueRepository,
    ) -> Result<Vec<AssetId>, DomainError> {
        // 1. 获取上游资产信息
        let upstream_asset = repo.find_by_id(&asset_id).await?
            .ok_or(DomainError::AssetNotFound(asset_id))?;

        // 2. 查找直接下游资产（谁依赖我）
        // 查询方向：WHERE target_id = 上游资产ID
        let downstream = repo.find_direct_downstream(&asset_id).await?;

        // 3. 对每个下游资产标记 Dirty
        let mut affected = Vec::new();
        for down_id in downstream {
            let mut downstream_asset = repo.find_by_id(&down_id).await?
                .ok_or(DomainError::AssetNotFound(down_id))?;

            let old_state = downstream_asset.current_state;
            if old_state.is_archived() {
                // Archived 是终态：保留在依赖图中，但不再接收 Dirty
                continue;
            }
            old_state
                .validate_transition(AssetState::Dirty)
                .map_err(|_e| DomainError::InvalidStateTransition {
                    from: format!("{:?}", old_state),
                    to: format!("{:?}", AssetState::Dirty),
                })?;

            // 边界检查：项目级资产只能依赖同项目/同组织的资产
            self.validate_dependency_boundary(&upstream_asset, &downstream_asset)?;

            // 计算影响等级
            let impact_level = self.calculate_impact(&asset_id, &down_id)?;

            // 获取当前有效依赖版本（用于 DirtyQueueEntry）
            let current_effective = repo
                .get_effective_version(&down_id, &asset_id)
                .await?;

            // 创建或更新 Dirty 队列条目
            let dirty_entry = DirtyQueueEntry {
                id: Uuid::new_v4(),
                asset_id: down_id,
                upstream_asset_id: asset_id,
                upstream_version: new_version.clone(),
                upstream_old_version: current_effective,
                impact_level,
                since: Utc::now(),
                resolved: false,
                resolved_at: None,
            };

            // 插入或更新 Dirty 队列（upsert 逻辑在 repo 中实现）
            dirty_queue_repo.upsert(&dirty_entry).await?;

            // 更新资产状态为 Dirty（仅设置状态标签）
            downstream_asset.current_state = AssetState::Dirty;
            downstream_asset.updated_at = Utc::now();
            repo.save(&downstream_asset).await?;

            affected.push(down_id);

            // 发布状态变更事件
            self.event_publisher.publish(DomainEvent::AssetStateChanged {
                asset_id: down_id,
                old_state,
                new_state: AssetState::Dirty,
                triggered_by: asset_id,
            }).await?;
        }

        Ok(affected)
    }

    /// 处理手工 Clean 事件
    /// 规则：不触发下游 Dirty
    pub async fn on_manual_clean(
        &self,
        asset_id: AssetId,
        resolutions: Vec<DirtyResolution>,
        repo: &dyn AssetRepository,
        dirty_queue_repo: &dyn DirtyQueueRepository,
    ) -> Result<(), DomainError> {
        let mut asset = repo.find_by_id(&asset_id).await?
            .ok_or(DomainError::AssetNotFound(asset_id))?;

        // 验证当前状态为 Dirty
        if asset.current_state != AssetState::Dirty {
            return Err(DomainError::InvalidStateTransition {
                from: format!("{:?}", asset.current_state),
                to: "Clean".to_string(),
            });
        }

        // 更新有效依赖基线
        for resolution in resolutions {
            // 更新依赖关系表中的 effective_version
            repo.update_effective_version(
                &asset_id,
                &resolution.upstream_asset_id,
                resolution.to_version.clone(),
                resolution.reviewed_by.clone(),
                EffectiveUpdateReason::ManualClean,
            ).await?;

            // 标记 Dirty 队列为已解决
            dirty_queue_repo.resolve(&asset_id, &resolution.upstream_asset_id).await?;

            // 记录处理日志
            repo.save_dirty_resolution(&DirtyResolutionLog {
                id: Uuid::new_v4(),
                asset_id,
                asset_version: asset.current_version.clone(),
                upstream_asset_id: resolution.upstream_asset_id,
                from_version: resolution.from_version,
                to_version: resolution.to_version,
                action: ResolutionAction::ManualClean,
                review_result: resolution.review_result,
                comment: resolution.comment,
                reviewed_by: resolution.reviewed_by.clone(),
                reviewed_at: Utc::now(),
            }).await?;
        }

        // 检查是否还有未解决的 Dirty 条目
        let unresolved_count = dirty_queue_repo
            .count_unresolved(&asset_id)
            .await?;

        // 如果没有未解决的 Dirty，状态恢复 Clean
        if unresolved_count == 0 {
            asset.current_state = AssetState::Clean;
            asset.updated_at = Utc::now();
            repo.save(&asset).await?;
        }

        // 发布事件（但不触发下游）
        self.event_publisher.publish(DomainEvent::AssetManuallyCleaned {
            asset_id,
            resolutions,
        }).await?;

        Ok(())
    }

    fn validate_dependency_boundary(
        &self,
        upstream: &AssetInstance,
        downstream: &AssetInstance,
    ) -> Result<(), DomainError> {
        let boundary = DependencyBoundaryContext {
            source_level: downstream.level,
            source_project_id: downstream.project_id,
            source_org_id: downstream.organization_id,
            target_level: upstream.level,
            target_project_id: upstream.project_id,
            target_org_id: upstream.organization_id,
        };
        
        boundary.validate()
            .map_err(|e| DomainError::DependencyBoundaryViolation(e.to_string()))
    }

    fn calculate_impact(
        &self,
        _upstream: &AssetId,
        _downstream: &AssetId,
    ) -> Result<ImpactLevel, DomainError> {
        // 根据资产类型、变更内容计算影响等级
        // 简化实现：返回 Medium
        Ok(ImpactLevel::Medium)
    }
}

/// Dirty 队列仓库接口
#[async_trait]
pub trait DirtyQueueRepository: Send + Sync {
    /// 插入或更新（如果已存在未解决的，更新版本信息）
    async fn upsert(&self, entry: &DirtyQueueEntry) -> Result<(), RepositoryError>;
    /// 标记为已解决
    async fn resolve(&self, asset_id: &AssetId, upstream_id: &AssetId) -> Result<(), RepositoryError>;
    /// 统计未解决的 Dirty 数量
    async fn count_unresolved(&self, asset_id: &AssetId) -> Result<i64, RepositoryError>;
    /// 查询资产的未解决 Dirty 条目
    async fn find_unresolved_by_asset(&self, asset_id: &AssetId) -> Result<Vec<DirtyQueueEntry>, RepositoryError>;
    /// 查询资产的 Dirty 条目（包含已解决）
    async fn find_by_asset(&self, asset_id: &AssetId) -> Result<Vec<DirtyQueueEntry>, RepositoryError>;
}
```

### 4.3 版本建议服务

```rust
/// 版本建议服务
pub struct VersionSuggester;

impl VersionSuggester {
    /// 分析变更内容并建议版本类型（仅适用于 SemVer 策略的资产类型）
    pub fn suggest_version_type(
        &self,
        asset_type: &AssetType,
        previous_version: &VersionIdentifier,
        metadata_changes: &MetadataDiff,
        dependency_changes: &[DependencyChange],
    ) -> Option<ChangeType> {
        // 仅当资产类型使用 SemVer 策略时才提供版本建议
        if asset_type.version_strategy != VersionStrategy::SemVer {
            return None;
        }

        // 尝试解析为 SemVer
        let semver = match SemVersion::parse(previous_version.as_str()) {
            Ok(v) => v,
            Err(_) => return None, // 无法解析则返回 None
        };

        // 检查是否有破坏性变更
        if self.has_breaking_changes(metadata_changes) {
            return Some(ChangeType::Major);
        }

        // 检查是否有新增功能
        if self.has_new_features(metadata_changes) {
            return Some(ChangeType::Minor);
        }

        // 检查依赖是否有重大变更
        if self.has_major_dependency_changes(dependency_changes) {
            return Some(ChangeType::Minor);
        }

        // 默认为 Patch
        Some(ChangeType::Patch)
    }

    /// 生成下一个版本号（仅适用于 SemVer）
    pub fn generate_next_version(
        &self,
        current: &VersionIdentifier,
        change_type: ChangeType,
    ) -> Result<VersionIdentifier, VersionError> {
        // 解析当前版本为 SemVersion
        let semver = SemVersion::parse(current.as_str())?;
        // 使用 SemVersion 的 suggest_next 方法生成下一个版本
        let next = semver.suggest_next(change_type);
        Ok(VersionIdentifier::from_semver(&next))
    }

    fn has_breaking_changes(&self, diff: &MetadataDiff) -> bool {
        // 检查元数据中的 breaking 标记
        diff.has_field("breaking_change") || diff.has_field("api_change")
    }

    fn has_new_features(&self, diff: &MetadataDiff) -> bool {
        diff.has_field("new_feature") || diff.has_field("enhancement")
    }

    fn has_major_dependency_changes(&self, changes: &[DependencyChange]) -> bool {
        changes.iter().any(|c| c.is_major_upgrade())
    }
}
```

---

## 4.5 安全与权限模型

### 4.5.1 认证与授权

```rust
/// 认证主体
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthPrincipal {
    /// 主体ID（用户ID或服务账号ID）
    pub id: String,
    /// 主体类型
    pub principal_type: PrincipalType,
    /// 所属组织
    pub organization_id: OrganizationId,
    /// 角色列表
    pub roles: Vec<Role>,
    /// 可访问的项目列表（组织管理员和系统管理员不需要逐项列出）
    pub project_memberships: Vec<ProjectId>,
    /// API Key 或 Token 元数据
    pub metadata: AuthMetadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrincipalType {
    User,           // 普通用户
    ServiceAccount, // 服务账号（Git Hooks, CI/CD）
    ApiKey,         // API Key
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    SystemAdmin,      // 系统管理员
    OrgAdmin,         // 组织管理员
    ProjectAdmin,     // 项目管理员
    Developer,        // 开发人员
    Reader,           // 只读访问
    AiAgent,          // AI Agent（受限权限）
}

/// 权限级别
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Permission {
    // 资产类型管理
    AssetTypeCreate, AssetTypeRead, AssetTypeUpdate, AssetTypeDelete,
    // 资产实例管理
    AssetCreate, AssetRead, AssetUpdate, AssetDelete,
    // 版本管理
    VersionPublish, VersionRead,
    // 依赖管理
    DependencyCreate, DependencyRead, DependencyDelete,
    // 状态管理
    StateManualClean, StateArchive, StateRefresh,
    // 查询
    QueryAssets, QueryImpactAnalysis, QueryVirtualContext,
}

impl Role {
    /// 获取角色拥有的权限
    pub fn permissions(&self) -> Vec<Permission> {
        match self {
            Role::SystemAdmin => vec![/* 所有权限 */],
            Role::OrgAdmin => vec![
                Permission::AssetTypeCreate, Permission::AssetTypeRead, Permission::AssetTypeUpdate,
                Permission::AssetCreate, Permission::AssetRead, Permission::AssetUpdate, Permission::AssetDelete,
                Permission::VersionPublish, Permission::VersionRead,
                Permission::DependencyCreate, Permission::DependencyRead, Permission::DependencyDelete,
                Permission::StateManualClean, Permission::StateArchive, Permission::StateRefresh,
                Permission::QueryAssets, Permission::QueryImpactAnalysis, Permission::QueryVirtualContext,
            ],
            Role::ProjectAdmin => vec![
                Permission::AssetRead, Permission::AssetCreate, Permission::AssetUpdate,
                Permission::VersionPublish, Permission::VersionRead,
                Permission::DependencyCreate, Permission::DependencyRead,
                Permission::StateManualClean, Permission::StateRefresh,
                Permission::QueryAssets, Permission::QueryImpactAnalysis,
            ],
            Role::Developer => vec![
                Permission::AssetRead, Permission::AssetCreate,
                Permission::VersionPublish, Permission::VersionRead,
                Permission::DependencyCreate, Permission::DependencyRead,
                Permission::StateManualClean, Permission::StateRefresh,
                Permission::QueryAssets, Permission::QueryImpactAnalysis, Permission::QueryVirtualContext,
            ],
            Role::Reader => vec![
                Permission::AssetTypeRead,
                Permission::AssetRead, Permission::VersionRead,
                Permission::DependencyRead,
                Permission::QueryAssets, Permission::QueryImpactAnalysis,
            ],
            Role::AiAgent => vec![
                // AI Agent 只有读权限 + 虚拟上下文创建
                Permission::AssetRead, Permission::VersionRead, Permission::DependencyRead,
                Permission::QueryAssets, Permission::QueryImpactAnalysis, Permission::QueryVirtualContext,
            ],
        }
    }
}

/// 权限校验服务
pub struct AuthorizationService;

impl AuthorizationService {
    /// 校验权限
    pub fn check(
        principal: &AuthPrincipal,
        permission: Permission,
        resource_org: OrganizationId,
        resource_project: Option<ProjectId>,
    ) -> Result<(), AuthorizationError> {
        // 1. 组织边界检查
        if principal.organization_id != resource_org {
            return Err(AuthorizationError::CrossOrganizationAccessDenied);
        }

        // 2. 权限检查
        let has_permission = principal.roles.iter()
            .any(|role| role.permissions().contains(&permission));
        
        if !has_permission {
            return Err(AuthorizationError::PermissionDenied {
                required: permission,
                roles: principal.roles.clone(),
            });
        }

        // 3. 项目级资源检查（非组织管理员必须属于项目）
        if let Some(project_id) = resource_project {
            if !principal.roles.contains(&Role::OrgAdmin) && 
               !principal.roles.contains(&Role::SystemAdmin) {
                if !principal.project_memberships.contains(&project_id) {
                    return Err(AuthorizationError::ProjectAccessDenied(project_id));
                }
            }
        }

        Ok(())
    }
}
```

### 4.5.2 审计日志

```rust
/// 审计日志条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditLog {
    pub id: Uuid,
    /// 操作时间
    pub timestamp: DateTime<Utc>,
    /// 操作主体
    pub principal_id: String,
    pub principal_type: PrincipalType,
    /// 操作类型
    pub action: AuditAction,
    /// 资源类型
    pub resource_type: String,
    /// 资源ID
    pub resource_id: String,
    /// 操作详情
    pub details: JsonValue,
    /// 操作结果
    pub result: AuditResult,
    /// 错误信息（如果失败）
    pub error_message: Option<String>,
    /// 客户端IP
    pub client_ip: Option<String>,
    /// 请求ID
    pub request_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuditAction {
    Create, Update, Delete, Publish, ManualClean, Archive,
    Query, // 敏感查询也需要审计
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuditResult {
    Success, Failure,
}

/// 审计日志服务
pub trait AuditLogService: Send + Sync {
    async fn log(&self, entry: AuditLog) -> Result<(), AuditError>;
}
```

### 4.5.3 速率限制

```rust
use std::time::Duration;
use redis::AsyncCommands;

/// 速率限制配置
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    /// 每秒请求数
    pub requests_per_second: u32,
    /// 突发容量
    pub burst_capacity: u32,
    /// 限制窗口
    pub window: Duration,
}

impl RateLimitConfig {
    /// 默认配置
    pub fn default_for_role(role: &Role) -> Self {
        match role {
            Role::SystemAdmin => Self {
                requests_per_second: 1000,
                burst_capacity: 5000,
                window: Duration::from_secs(60),
            },
            Role::AiAgent => Self {
                requests_per_second: 100,  // AI Agent 更严格的限制
                burst_capacity: 200,
                window: Duration::from_secs(60),
            },
            _ => Self {
                requests_per_second: 50,
                burst_capacity: 100,
                window: Duration::from_secs(60),
            },
        }
    }
}

/// 速率限制服务（基于 Redis Token Bucket）
pub struct RateLimitService {
    redis: redis::aio::MultiplexedConnection,
}

impl RateLimitService {
    /// 检查是否允许请求
    pub async fn allow(&self, key: &str, config: &RateLimitConfig) -> Result<bool, RateLimitError> {
        let mut conn = self.redis.clone();
        
        // Token bucket 实现
        let bucket_key = format!("rate_limit:{}", key);
        let tokens_key = format!("{}:tokens", bucket_key);
        let last_update_key = format!("{}:last_update", bucket_key);
        
        let now = Utc::now().timestamp_millis();
        let window_ms = config.window.as_millis() as i64;
        
        // 使用 Lua 脚本保证原子性
        let script = r#"
            local tokens_key = KEYS[1]
            local last_update_key = KEYS[2]
            local rate = tonumber(ARGV[1])
            local capacity = tonumber(ARGV[2])
            local now = tonumber(ARGV[3])
            
            local tokens = redis.call('GET', tokens_key)
            if not tokens then tokens = capacity end
            tokens = tonumber(tokens)
            
            local last_update = redis.call('GET', last_update_key)
            if not last_update then last_update = now end
            last_update = tonumber(last_update)
            
            local delta = math.max(0, now - last_update)
            tokens = math.min(capacity, tokens + delta * rate / 1000)
            
            if tokens >= 1 then
                tokens = tokens - 1
                redis.call('SET', tokens_key, tokens, 'PX', ARGV[4])
                redis.call('SET', last_update_key, now, 'PX', ARGV[4])
                return 1
            else
                redis.call('SET', tokens_key, tokens, 'PX', ARGV[4])
                redis.call('SET', last_update_key, now, 'PX', ARGV[4])
                return 0
            end
        "#;
        
        let allowed: i32 = redis::Script::new(script)
            .key(&tokens_key)
            .key(&last_update_key)
            .arg(config.requests_per_second)
            .arg(config.burst_capacity)
            .arg(now)
            .arg(window_ms)
            .invoke_async(&mut conn)
            .await?;
        
        Ok(allowed == 1)
    }
}
```

---

## 5. 技术栈选型

### 5.1 核心依赖

```toml
# Cargo.toml
[package]
name = "adam"
version = "0.1.0"
edition = "2024"
rust-version = "1.85"

[dependencies]
# 异步运行时
tokio = { version = "1.44", features = ["full"] }
tokio-util = "0.7"
async-trait = "0.1"

# Web 框架
axum = "0.8"
tower = "0.5"
tower-http = "0.6"

# 序列化
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
toml = "0.8"

# 数据库
sqlx = { version = "0.8", features = ["runtime-tokio", "postgres", "uuid", "chrono", "json", "migrate"] }
migration = "0.3"

# 缓存与消息队列
redis = { version = "0.29", features = ["tokio-comp"] }

# Graph 算法
petgraph = "0.7"

# 语义化版本
semver = "1.0"

# UUID
uuid = { version = "1.16", features = ["v4", "serde"] }

# 时间处理
chrono = { version = "0.4", features = ["serde"] }

# 错误处理
thiserror = "2.0"
anyhow = "1.0"

# 日志与追踪
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

# 配置
config = "0.15"

# 验证
validator = { version = "0.20", features = ["derive"] }

# JSON Schema
schemars = "0.8"
jsonschema = "0.29"

# HTTP 客户端
reqwest = { version = "0.12", features = ["json"] }

# CLI
clap = { version = "4.5", features = ["derive"] }

# MCP SDK
rmcp = "0.3"

# 测试
mockall = "0.13"
```

### 5.2 开发工具

```toml
# .cargo/config.toml
[build]
target-dir = "target"

[env]
RUST_LOG = "info,sqlx=warn"
RUST_BACKTRACE = "1"

# rustfmt.toml
edition = "2024"
max_width = 100
tab_spaces = 4
```

---

## 6. 数据库设计

### 6.1 ER 图

```
┌──────────────────┐      ┌──────────────────┐
│ organizations    │      │ projects         │
├──────────────────┤      ├──────────────────┤
│ id (PK)          │◄─────│ id (PK)          │
│ name             │      │ org_id (FK)      │
│ ...              │      │ name             │
└──────────────────┘      └──────────────────┘
         │                         │
         │         ┌───────────────┘
         │         │
         ▼         ▼
┌──────────────────┐      ┌──────────────────┐
│    asset_types   │      │ asset_instances  │
├──────────────────┤      ├──────────────────┤
│ id (PK)          │      │ id (PK)          │
│ org_id (FK)      │◄─────│ type_id (FK)     │
│ name             │      │ org_id (FK)      │
│ version_strategy │      │ project_id (FK)  │
│ ...              │      │ level            │
└──────────────────┘      │ current_state    │
         │                │ idempotency_key  │
         │                │ archived_at      │
         ▼                │ ...              │
┌──────────────────┐      └──────────────────┘
│ dependency_rules │                  │
├──────────────────┤                  │
│ id (PK)          │                  │
│ org_id (FK)      │                  │
│ source_type_id   │                  ▼
│ target_type_id   │      ┌──────────────────┐      ┌──────────────────┐
│ ...              │      │ asset_dependencies│     │ dirty_queue      │
└──────────────────┘      ├──────────────────┤      ├──────────────────┤
                          │ id (PK)          │      │ id (PK)          │
                          │ source_id (FK)   │      │ asset_id (FK)    │
                          │ target_id (FK)   │      │ upstream_id (FK) │
                          │ declared_version │      │ impact_level     │
                          │ effective_version│      │ resolved         │
                          └──────────────────┘      └──────────────────┘
                                   │
                                   ▼
                          ┌──────────────────┐      ┌──────────────────┐
                          │ asset_versions   │      │ audit_logs       │
                          ├──────────────────┤      ├──────────────────┤
                          │ id (PK)          │      │ id (PK)          │
                          │ instance_id (FK) │      │ principal_id     │
                          │ version_number   │      │ action           │
                          │ ...              │      │ resource_id      │
                          └──────────────────┘      │ ...              │
                                                    └──────────────────┘
                                                    ┌──────────────────┐
                                                    │ dirty_resolution │
                                                    │ _logs            │
                                                    ├──────────────────┤
                                                    │ id (PK)          │
                                                    │ asset_id (FK)    │
                                                    │ ...              │
                                                    └──────────────────┘
                                                    ┌──────────────────┐
                                                    │ virtual_instances│
                                                    ├──────────────────┤
                                                    │ id (PK)          │
                                                    │ ...              │
                                                    └──────────────────┘
                                                    ┌──────────────────┐
                                                    │ pipeline_runs    │
                                                    ├──────────────────┤
                                                    │ id (PK)          │
                                                    │ ...              │
                                                    └──────────────────┘
```

### 6.2 SQL Schema

```sql
-- 组织表
CREATE TABLE organizations (
    id UUID PRIMARY KEY,
    name VARCHAR(200) NOT NULL,
    description TEXT,
    settings JSONB NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- 项目表
CREATE TABLE projects (
    id UUID PRIMARY KEY,
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    name VARCHAR(200) NOT NULL,
    description TEXT,
    settings JSONB NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(organization_id, name)
);

-- 资产类型表
CREATE TABLE asset_types (
    id UUID PRIMARY KEY,
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    name VARCHAR(100) NOT NULL,
    display_name VARCHAR(200) NOT NULL,
    description TEXT,
    metadata_schema JSONB NOT NULL DEFAULT '{}',
    version_strategy VARCHAR(20) NOT NULL DEFAULT 'semver' 
        CHECK (version_strategy IN ('semver', 'external_ref', 'composite')),
    retention_policy JSONB NOT NULL DEFAULT '{}',
    icon VARCHAR(200),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(organization_id, name)
);

-- 依赖规则表
CREATE TABLE dependency_rules (
    id UUID PRIMARY KEY,
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    source_type_id UUID NOT NULL REFERENCES asset_types(id),
    target_type_id UUID NOT NULL REFERENCES asset_types(id),
    relationship VARCHAR(50) NOT NULL DEFAULT 'depends_on',
    is_transitive BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(organization_id, source_type_id, target_type_id)
);

-- 资产实例表
CREATE TABLE asset_instances (
    id UUID PRIMARY KEY,
    type_id UUID NOT NULL REFERENCES asset_types(id),
    organization_id UUID NOT NULL REFERENCES organizations(id),
    name VARCHAR(500) NOT NULL,
    external_ref TEXT NOT NULL,
    source VARCHAR(50) NOT NULL,
    level VARCHAR(20) NOT NULL CHECK (level IN ('project', 'organization')),
    project_id UUID REFERENCES projects(id),
    current_version VARCHAR(200) NOT NULL,  -- 支持各种版本格式
    current_state VARCHAR(20) NOT NULL CHECK (current_state IN ('clean', 'dirty', 'archived')),
    -- 归档状态字段（Archived 状态时有效）
    archived_at TIMESTAMPTZ,
    archived_reason TEXT,
    metadata JSONB NOT NULL DEFAULT '{}',
    publisher VARCHAR(200) NOT NULL,
    assignees TEXT[] NOT NULL DEFAULT '{}',
    -- 幂等键：标准化格式 "source:org_id:project_id?:external_ref"
    idempotency_key VARCHAR(1000) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    
    -- 约束：项目级必须有 project_id，组织级必须没有
    CONSTRAINT project_level_check CHECK (
        (level = 'project' AND project_id IS NOT NULL) OR
        (level = 'organization' AND project_id IS NULL)
    ),
    -- 幂等键唯一约束（跨组织唯一，因为 idempotency_key 包含 org_id）
    UNIQUE(idempotency_key)
);

-- =============================================
-- 触发器函数定义（必须在触发器之前创建）
-- =============================================

-- 检查资产实例的 type_id 是否与 organization_id 属于同一组织
CREATE OR REPLACE FUNCTION check_asset_instance_type_org()
RETURNS TRIGGER AS $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM asset_types at
        WHERE at.id = NEW.type_id
        AND at.organization_id != NEW.organization_id
    ) THEN
        RAISE EXCEPTION 'Asset type organization mismatch: type_id % does not belong to organization %', 
            NEW.type_id, NEW.organization_id;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- 检查资产实例的 project_id 是否与 organization_id 属于同一组织
CREATE OR REPLACE FUNCTION check_asset_instance_project_org()
RETURNS TRIGGER AS $$
BEGIN
    IF NEW.project_id IS NOT NULL AND EXISTS (
        SELECT 1 FROM projects p
        WHERE p.id = NEW.project_id
        AND p.organization_id != NEW.organization_id
    ) THEN
        RAISE EXCEPTION 'Project organization mismatch: project_id % does not belong to organization %', 
            NEW.project_id, NEW.organization_id;
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- 检查依赖关系的 source_id 和 target_id 是否在同一组织，且满足层级约束
CREATE OR REPLACE FUNCTION check_dependencies_same_org()
RETURNS TRIGGER AS $$
DECLARE
    source_org UUID;
    target_org UUID;
    source_project UUID;
    target_project UUID;
    source_level VARCHAR(20);
    target_level VARCHAR(20);
BEGIN
    -- 获取 source 和 target 的组织、项目、层级信息
    SELECT organization_id, project_id, level 
    INTO source_org, source_project, source_level 
    FROM asset_instances WHERE id = NEW.source_id;
    
    SELECT organization_id, project_id, level 
    INTO target_org, target_project, target_level 
    FROM asset_instances WHERE id = NEW.target_id;
    
    -- 检查组织边界
    IF source_org != target_org THEN
        RAISE EXCEPTION 'Cross-organization dependency not allowed: source % (org %) vs target % (org %)', 
            NEW.source_id, source_org, NEW.target_id, target_org;
    END IF;
    
    -- 检查项目和层级边界（BR-008）
    -- 规则：
    -- 1. 项目级 -> 项目级：必须同项目（不跨项目）
    -- 2. 项目级 -> 组织级：禁止（组织级资产仅通过查询上下文合并）
    -- 3. 组织级 -> 组织级：允许（同组织）
    -- 4. 组织级 -> 项目级：禁止
    
    -- 规则1：项目级->项目级 必须同项目
    IF source_level = 'project' AND target_level = 'project' AND source_project != target_project THEN
        RAISE EXCEPTION 'Cross-project dependency not allowed: source % (project %) vs target % (project %)', 
            NEW.source_id, source_project, NEW.target_id, target_project;
    END IF;

    -- 规则2：项目级不能持久化依赖组织级
    IF source_level = 'project' AND target_level = 'organization' THEN
        RAISE EXCEPTION 'Project-level asset cannot persist dependency on organization-level asset: source % vs target %',
            NEW.source_id, NEW.target_id;
    END IF;
    
    -- 规则4：组织级只能依赖组织级
    IF source_level = 'organization' AND target_level != 'organization' THEN
        RAISE EXCEPTION 'Organization-level asset can only depend on organization-level assets: source % (level %) vs target % (level %)', 
            NEW.source_id, source_level, NEW.target_id, target_level;
    END IF;
    
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- 检查依赖规则的 source_type_id 和 target_type_id 是否在同一组织
CREATE OR REPLACE FUNCTION check_dependency_rules_same_org()
RETURNS TRIGGER AS $$
DECLARE
    source_org UUID;
    target_org UUID;
BEGIN
    SELECT organization_id INTO source_org FROM asset_types WHERE id = NEW.source_type_id;
    SELECT organization_id INTO target_org FROM asset_types WHERE id = NEW.target_type_id;
    
    IF source_org != target_org THEN
        RAISE EXCEPTION 'Cross-organization dependency rule not allowed: source_type % (org %) vs target_type % (org %)', 
            NEW.source_type_id, source_org, NEW.target_type_id, target_org;
    END IF;
    
    -- 还需检查规则自身的 organization_id 与类型一致
    IF NEW.organization_id != source_org THEN
        RAISE EXCEPTION 'Dependency rule organization mismatch: rule org % vs type org %', 
            NEW.organization_id, source_org;
    END IF;
    
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- =============================================
-- 约束触发器（引用上面的函数）
-- =============================================

-- 资产实例组织一致性约束
-- 确保 asset_instances.type_id 与 organization_id 属于同一组织
CREATE CONSTRAINT TRIGGER trg_asset_instances_type_org_consistency
    AFTER INSERT OR UPDATE ON asset_instances
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    EXECUTE FUNCTION check_asset_instance_type_org();

-- 确保 asset_instances.project_id 与 organization_id 属于同一组织
CREATE CONSTRAINT TRIGGER trg_asset_instances_project_org_consistency
    AFTER INSERT OR UPDATE ON asset_instances
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    EXECUTE FUNCTION check_asset_instance_project_org();

-- Dirty 队列表（存储 Dirty 状态详情）
CREATE TABLE dirty_queue (
    id UUID PRIMARY KEY,
    -- 受影响的资产（下游）
    asset_id UUID NOT NULL REFERENCES asset_instances(id) ON DELETE CASCADE,
    -- 导致 Dirty 的上游资产
    upstream_asset_id UUID NOT NULL REFERENCES asset_instances(id),
    -- 上游新版本
    upstream_version VARCHAR(200) NOT NULL,
    -- 上游旧版本（当前有效基线）
    upstream_old_version VARCHAR(200) NOT NULL,
    -- 影响等级
    impact_level VARCHAR(20) NOT NULL CHECK (impact_level IN ('low', 'medium', 'high', 'critical')),
    -- Dirty 开始时间
    since TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- 是否已处理
    resolved BOOLEAN NOT NULL DEFAULT false,
    -- 处理时间
    resolved_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- 部分唯一索引：仅限制未解决的 (resolved = false) 条目唯一
-- 允许同一资产和上游组合有多个已解决的记录，但最多一个未解决的
CREATE UNIQUE INDEX idx_dirty_queue_unresolved 
ON dirty_queue (asset_id, upstream_asset_id) 
WHERE resolved = false;

-- 依赖关系表
-- 依赖方向：source_id (下游) -> target_id (上游)
CREATE TABLE asset_dependencies (
    id UUID PRIMARY KEY,
    source_id UUID NOT NULL REFERENCES asset_instances(id) ON DELETE CASCADE,  -- 下游（依赖方）
    target_id UUID NOT NULL REFERENCES asset_instances(id) ON DELETE RESTRICT, -- 上游（被依赖方）
    relationship VARCHAR(50) NOT NULL DEFAULT 'depends_on',
    declared_version VARCHAR(200) NOT NULL,  -- 发布时声明
    effective_version VARCHAR(200) NOT NULL, -- 当前有效基线
    effective_updated_by VARCHAR(200) NOT NULL,
    effective_updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    effective_reason VARCHAR(50) NOT NULL CHECK (effective_reason IN ('publish', 'manual_clean')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(source_id, target_id)
);

-- 依赖关系组织边界约束：source_id 和 target_id 必须在同一组织内
CREATE CONSTRAINT TRIGGER trg_asset_dependencies_same_org
    AFTER INSERT OR UPDATE ON asset_dependencies
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    EXECUTE FUNCTION check_dependencies_same_org();

-- 依赖规则组织一致性约束
CREATE CONSTRAINT TRIGGER trg_dependency_rules_same_org
    AFTER INSERT OR UPDATE ON dependency_rules
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW
    EXECUTE FUNCTION check_dependency_rules_same_org();

-- 版本发布记录表
CREATE TABLE asset_versions (
    id UUID PRIMARY KEY,
    instance_id UUID NOT NULL REFERENCES asset_instances(id) ON DELETE CASCADE,
    version_number VARCHAR(200) NOT NULL,
    metadata JSONB NOT NULL DEFAULT '{}',
    dependencies JSONB NOT NULL DEFAULT '[]',
    release_notes TEXT,
    suggested_type VARCHAR(20) CHECK (suggested_type IN ('major', 'minor', 'patch')),
    released_by VARCHAR(200) NOT NULL,
    released_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(instance_id, version_number)
);

-- Dirty 处理日志表
CREATE TABLE dirty_resolution_logs (
    id UUID PRIMARY KEY,
    asset_id UUID NOT NULL REFERENCES asset_instances(id),
    asset_version VARCHAR(200) NOT NULL,
    upstream_asset_id UUID NOT NULL REFERENCES asset_instances(id),
    from_version VARCHAR(200) NOT NULL,
    to_version VARCHAR(200) NOT NULL,
    action VARCHAR(50) NOT NULL CHECK (action IN ('manual_clean', 'republish')),
    review_result VARCHAR(50) NOT NULL CHECK (review_result IN ('no_impact', 'updated')),
    comment TEXT,
    reviewed_by VARCHAR(200) NOT NULL,
    reviewed_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- 审计日志表
CREATE TABLE audit_logs (
    id UUID PRIMARY KEY,
    timestamp TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    principal_id VARCHAR(200) NOT NULL,
    principal_type VARCHAR(50) NOT NULL CHECK (principal_type IN ('user', 'service_account', 'api_key')),
    action VARCHAR(50) NOT NULL,
    resource_type VARCHAR(100) NOT NULL,
    resource_id VARCHAR(200) NOT NULL,
    details JSONB NOT NULL DEFAULT '{}',
    result VARCHAR(20) NOT NULL CHECK (result IN ('success', 'failure')),
    error_message TEXT,
    client_ip INET,
    request_id VARCHAR(200) NOT NULL,
    organization_id UUID REFERENCES organizations(id)
);

-- 虚拟实例表（短期存在）
CREATE TABLE virtual_instances (
    id UUID PRIMARY KEY,
    target_type_id UUID NOT NULL REFERENCES asset_types(id),
    anchor_ids UUID[] NOT NULL,
    session_id VARCHAR(200) NOT NULL,
    organization_id UUID NOT NULL REFERENCES organizations(id),
    project_id UUID NOT NULL REFERENCES projects(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL
);

-- 流水线执行记录表
CREATE TABLE pipeline_runs (
    id UUID PRIMARY KEY,
    pipeline_asset_id UUID NOT NULL REFERENCES asset_instances(id),
    organization_id UUID NOT NULL REFERENCES organizations(id),
    project_id UUID NOT NULL REFERENCES projects(id),
    run_id VARCHAR(200) NOT NULL,
    commit_sha VARCHAR(100),
    status VARCHAR(50) NOT NULL CHECK (status IN ('success', 'failed', 'cancelled', 'running')),
    trigger VARCHAR(100) NOT NULL,
    started_at TIMESTAMPTZ NOT NULL,
    finished_at TIMESTAMPTZ,
    summary JSONB NOT NULL DEFAULT '{}',
    external_log_ref TEXT,
    retained_until TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    
    UNIQUE(pipeline_asset_id, run_id)
);

-- =============================================
-- 索引
-- =============================================
CREATE INDEX idx_asset_instances_org ON asset_instances(organization_id);
CREATE INDEX idx_asset_instances_project ON asset_instances(project_id);
CREATE INDEX idx_asset_instances_type ON asset_instances(type_id);
CREATE INDEX idx_asset_instances_state ON asset_instances(current_state);
CREATE INDEX idx_asset_instances_idempotency ON asset_instances(idempotency_key);

CREATE INDEX idx_dirty_queue_asset ON dirty_queue(asset_id);
CREATE INDEX idx_dirty_queue_upstream ON dirty_queue(upstream_asset_id);
-- 注意：部分唯一索引（resolved=false）已在表定义后创建

-- 依赖方向索引：查询下游（谁依赖我）
CREATE INDEX idx_dependencies_target ON asset_dependencies(target_id);
-- 依赖方向索引：查询上游（我依赖谁）
CREATE INDEX idx_dependencies_source ON asset_dependencies(source_id);

CREATE INDEX idx_versions_instance ON asset_versions(instance_id);
CREATE INDEX idx_resolution_logs_asset ON dirty_resolution_logs(asset_id);
CREATE INDEX idx_virtual_expires ON virtual_instances(expires_at);
CREATE INDEX idx_pipeline_runs_asset ON pipeline_runs(pipeline_asset_id);

-- 审计日志索引
CREATE INDEX idx_audit_timestamp ON audit_logs(timestamp);
CREATE INDEX idx_audit_principal ON audit_logs(principal_id);
CREATE INDEX idx_audit_resource ON audit_logs(resource_type, resource_id);
CREATE INDEX idx_audit_org ON audit_logs(organization_id);
```

---

## 7. API 设计

### 7.1 REST API

```rust
use axum::{
    routing::{get, post, put, delete},
    Router,
    extract::{Path, Query, State},
    Json,
};

/// API 路由配置
pub fn create_router(state: AppState) -> Router {
    // 认证中间件层
    let auth_layer = middleware::from_fn_with_state(state.clone(), auth_middleware);
    
    Router::new()
        // 公开端点（不需要认证）
        .route("/api/v1/health", get(health_check))
        // 受保护端点（需要认证）
        .merge(protected_routes(state).layer(auth_layer))
}

/// 受保护的路由 - 需要认证
fn protected_routes(state: AppState) -> Router {
    Router::new()
        // 资产类型管理
        .route("/asset-types", post(create_asset_type))
        .route("/asset-types", get(list_asset_types))
        .route("/asset-types/:id", get(get_asset_type))
        .route("/asset-types/:id", put(update_asset_type))
        .route("/asset-types/:id", delete(delete_asset_type))
        // 依赖规则
        .route("/dependency-rules", post(create_dependency_rule))
        .route("/dependency-rules", get(list_dependency_rules))
        .route("/dependency-rules/:id", delete(delete_dependency_rule))
        // 资产实例
        .route("/assets", post(create_asset))
        .route("/assets", get(list_assets))
        .route("/assets/:id", get(get_asset))
        .route("/assets/:id", put(update_asset))
        .route("/assets/:id", delete(delete_asset))
        // 版本管理
        .route("/assets/:id/releases", post(publish_version))
        .route("/assets/:id/versions", get(list_versions))
        .route("/assets/:id/versions/:version", get(get_version))
        // 依赖关系
        .route("/assets/:id/dependencies", get(get_dependencies))
        .route("/assets/:id/dependency-graph", get(get_dependency_graph))
        // 状态管理
        .route("/assets/:id/manual-clean", post(manual_clean))
        .route("/assets/:id/archive", post(archive_asset))
        .route("/assets/:id/impact-analysis", get(impact_analysis))
        // 虚拟实例
        .route("/virtual-assets", post(create_virtual_asset))
        .route("/virtual-assets/:id/context", get(get_virtual_context))
        .route("/virtual-assets/:id", delete(delete_virtual_asset))
        // 内容获取
        .route("/assets/:id/content", get(get_asset_content))
        .route("/assets/:id/metadata", get(get_asset_metadata))
        .with_state(state)
}

/// 认证中间件：从请求头提取认证信息
async fn auth_middleware(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    // 从请求头提取认证信息
    let principal = extract_principal(&headers, &state)?;
    
    // 将认证主体存入请求扩展
    request.extensions_mut().insert(principal);
    
    Ok(next.run(request).await)
}

/// 从请求头提取认证主体
fn extract_principal(
    headers: &HeaderMap,
    state: &AppState,
) -> Result<AuthPrincipal, StatusCode> {
    // 1. 尝试从 Authorization 头提取 Bearer Token
    if let Some(auth_header) = headers.get("authorization") {
        if let Ok(auth_str) = auth_header.to_str() {
            if auth_str.starts_with("Bearer ") {
                let token = auth_str.trim_start_matches("Bearer ");
                return state.auth_service.validate_token(token);
            }
        }
    }
    
    // 2. 尝试从 X-API-Key 头提取
    if let Some(api_key) = headers.get("x-api-key") {
        if let Ok(key) = api_key.to_str() {
            return state.auth_service.validate_api_key(key);
        }
    }
    
    // 3. 尝试从 X-Service-Account 头提取（服务账号）
    if let Some(service_header) = headers.get("x-service-account") {
        if let Ok(account) = service_header.to_str() {
            return state.auth_service.validate_service_account(account);
        }
    }
    
    Err(StatusCode::UNAUTHORIZED)
}

// DTO 定义
#[derive(Debug, Deserialize, Validate)]
pub struct CreateAssetRequest {
    #[validate(length(min = 1, max = 500))]
    pub name: String,
    pub type_id: AssetTypeId,
    pub external_ref: String,
    pub source: AssetSource,
    pub level: AssetLevel,
    /// 项目级资产必须提供 project_id，组织级资产为 None
    pub project_id: Option<ProjectId>,
    pub metadata: JsonValue,
    pub dependencies: Vec<DependencyDeclaration>,
    pub release_notes: Option<String>,
    pub initial_version: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DependencyDeclaration {
    pub target_id: AssetId,
    pub relation: DependencyRelation,
}

#[derive(Debug, Deserialize)]
pub struct ListAssetsQuery {
    pub project_id: ProjectId,
    pub level: Option<AssetLevel>,
    pub asset_type: Option<AssetTypeId>,
    pub state: Option<AssetState>,
    pub assignee: Option<String>,
    pub start_date: Option<DateTime<Utc>>,
    pub end_date: Option<DateTime<Utc>>,
    pub keyword: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AssetResponse {
    pub id: AssetId,
    pub name: String,
    pub type_id: AssetTypeId,
    pub type_name: String,
    pub external_ref: String,
    pub source: AssetSource,
    pub level: AssetLevel,
    pub project_id: Option<ProjectId>,
    pub current_version: String,
    pub current_state: AssetState,
    pub metadata: JsonValue,
    pub publisher: String,
    pub assignees: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct DependencyGraphResponse {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

#[derive(Debug, Serialize)]
pub struct GraphNode {
    pub id: AssetId,
    pub name: String,
    pub asset_type: String,
    pub state: AssetState,
    pub version: String,
}

#[derive(Debug, Serialize)]
pub struct GraphEdge {
    pub source: AssetId,
    pub target: AssetId,
    pub relation: DependencyRelation,
}

#[derive(Debug, Serialize)]
pub struct ImpactAnalysisResponse {
    pub upstream: Vec<ImpactItem>,
    pub downstream: Vec<ImpactItem>,
}

#[derive(Debug, Serialize)]
pub struct ImpactItem {
    pub asset_id: AssetId,
    pub name: String,
    pub asset_type: String,
    pub current_state: AssetState,
    pub depth: i32,
}

#[derive(Debug, Serialize)]
pub struct VirtualContextResponse {
    pub target_type: String,
    pub dependencies: Vec<ContextDependencyItem>,
}

#[derive(Debug, Serialize)]
pub struct ContextDependencyItem {
    pub asset_type: String,
    pub asset_type_display: String,
    pub instance_id: AssetId,
    pub instance_name: String,
    pub version: String,
    pub external_ref: String,
}

/// 处理器示例：展示如何从请求中提取 AuthPrincipal 并检查权限
mod handlers {
    use super::*;
    use axum::{
        extract::{Extension, State},
        http::StatusCode,
        Extension as Ext,
    };
    
    /// 提取认证主体的辅助函数
    fn extract_auth(ext: &axum::http::Extensions) -> Result<AuthPrincipal, StatusCode> {
        ext.get::<AuthPrincipal>()
            .cloned()
            .ok_or(StatusCode::UNAUTHORIZED)
    }
    
    /// 示例：创建资产 - 需要 AssetCreate 权限
    pub async fn create_asset(
        State(state): State<AppState>,
        Extension(principal): Extension<AuthPrincipal>,
        Json(req): Json<CreateAssetRequest>,
    ) -> Result<Json<AssetResponse>, StatusCode> {
        // 1. 组织上下文来自认证主体；跨组织代创建需走单独的管理员接口
        let organization_id = principal.organization_id.clone();
        
        // 2. 如果是项目级资产，验证项目属于该组织
        if let Some(project_id) = &req.project_id {
            let project_org = state.project_service
                .get_organization(project_id)?;
            if project_org != organization_id {
                return Err(StatusCode::FORBIDDEN);
            }
        }
        
        // 3. 权限检查（组织边界已在上面验证，这里检查角色权限）
        AuthorizationService::check(
            &principal,
            Permission::AssetCreate,
            organization_id.clone(),
            req.project_id.clone(),
        ).map_err(|_| StatusCode::FORBIDDEN)?;
        
        // 4. 注入发布者信息（从认证主体）
        let cmd = CreateAssetCommand {
            name: req.name,
            type_id: req.type_id,
            external_ref: req.external_ref,
            source: req.source,
            level: req.level,
            project_id: req.project_id,
            organization_id, // 注入组织ID
            metadata: req.metadata,
            dependencies: req.dependencies,
            release_notes: req.release_notes,
            initial_version: req.initial_version,
            publisher: principal.id.clone(), // 从认证主体注入
        };
        
        let asset = state.asset_service.create(cmd).await?;
        Ok(Json(AssetResponse::from(asset)))
    }
    
    /// 示例：查询资产列表 - 需要 QueryAssets 权限
    pub async fn list_assets(
        State(state): State<AppState>,
        Extension(principal): Extension<AuthPrincipal>,
        Query(query): Query<ListAssetsQuery>,
    ) -> Result<Json<PaginatedResponse<AssetResponse>>, StatusCode> {
        // 1. 获取项目所属组织
        let organization_id = state.project_service
            .get_organization(&query.project_id)?;
        
        // 2. 权限检查
        AuthorizationService::check(
            &principal,
            Permission::QueryAssets,
            organization_id,
            Some(query.project_id),
        ).map_err(|_| StatusCode::FORBIDDEN)?;
        
        // 3. 执行查询
        let assets = state.asset_service.list(
            organization_id,
            &query.project_id,
            &query,
        ).await?;
        
        Ok(Json(PaginatedResponse::from(assets)))
    }
    
    /// 示例：发布版本 - 需要 VersionPublish 权限
    pub async fn publish_version(
        State(state): State<AppState>,
        Extension(principal): Extension<AuthPrincipal>,
        Path(id): Path<AssetId>,
        Json(req): Json<PublishVersionRequest>,
    ) -> Result<Json<VersionResponse>, StatusCode> {
        // 1. 获取资产信息以确定组织和项目
        let asset = state.asset_service.get(&id).await?;
        
        // 2. 权限检查
        AuthorizationService::check(
            &principal,
            Permission::VersionPublish,
            asset.organization_id,
            asset.project_id,
        ).map_err(|_| StatusCode::FORBIDDEN)?;
        
        // 3. 执行发布
        let cmd = PublishAssetCommand {
            asset_id: id,
            release_notes: req.release_notes,
            suggested_version: req.version,
            dependencies: req.dependencies,
            publisher: principal.id, // 从认证主体注入
        };
        
        let version = state.version_service.publish(cmd).await?;
        Ok(Json(VersionResponse::from(version)))
    }
    
    /// 示例：手动清理 - 需要 StateManualClean 权限
    pub async fn manual_clean(
        State(state): State<AppState>,
        Extension(principal): Extension<AuthPrincipal>,
        Path(id): Path<AssetId>,
        Json(req): Json<ManualCleanRequest>,
    ) -> Result<Json<DirtyResolutionResponse>, StatusCode> {
        // 1. 获取资产信息
        let asset = state.asset_service.get(&id).await?;
        
        // 2. 权限检查
        AuthorizationService::check(
            &principal,
            Permission::StateManualClean,
            asset.organization_id,
            asset.project_id,
        ).map_err(|_| StatusCode::FORBIDDEN)?;
        
        // 3. 执行清理
        let result = state.state_service.manual_clean(
            id,
            req.upstream_versions,
            principal.id, // 从认证主体注入
            req.comment,
        ).await?;
        
        Ok(Json(DirtyResolutionResponse::from(result)))
    }
}
```

### 7.2 MCP Server 接口

```rust
use rmcp::{
    model::*,
    tool,
};

/// MCP Server 实现
pub struct AdamMcpServer {
    asset_service: Arc<dyn AssetService>,
    version_service: Arc<dyn VersionService>,
    impact_service: Arc<dyn ImpactAnalysisService>,
    virtual_service: Arc<dyn VirtualAssetService>,
    auth_service: Arc<dyn AuthService>,
    // 每个 MCP Session 的认证主体
    session_principal: AuthPrincipal,
}

impl AdamMcpServer {
    /// 创建新的 MCP Server 实例（每个连接）
    pub async fn new(
        services: ServiceRegistry,
        session_context: SessionContext,
    ) -> Result<Self, McpError> {
        // 1. 从 session_context 提取认证信息并验证
        let principal = services.auth_service()
            .validate_mcp_session(&session_context)
            .await?;
        
        Ok(Self {
            asset_service: services.asset_service(),
            version_service: services.version_service(),
            impact_service: services.impact_service(),
            virtual_service: services.virtual_service(),
            auth_service: services.auth_service(),
            session_principal: principal,
        })
    }
    
    /// 权限检查辅助函数
    fn check_permission(
        &self,
        permission: Permission,
        organization_id: OrganizationId,
        project_id: Option<ProjectId>,
    ) -> Result<(), McpError> {
        AuthorizationService::check(
            &self.session_principal,
            permission,
            organization_id,
            project_id,
        ).map_err(|e| McpError::permission_denied(e.to_string()))
    }
    
    /// 获取当前会话的组织上下文
    fn organization_id(&self) -> OrganizationId {
        self.session_principal.organization_id.clone()
    }
}

#[tool]
impl AdamMcpServer {
    /// 查询资产
    #[tool(name = "query_assets")]
    async fn query_assets(
        &self,
        #[tool(param(desc = "项目ID"))]
        project_id: String,
        #[tool(param(desc = "资产类型过滤", optional = true))]
        asset_type: Option<String>,
        #[tool(param(desc = "状态过滤", optional = true))]
        state: Option<String>,
        #[tool(param(desc = "关键词搜索", optional = true))]
        keyword: Option<String>,
    ) -> Result<CallToolResult, McpError> {
        // 1. 权限检查
        let org_id = self.organization_id();
        let project_id = ProjectId::parse(&project_id)?;
        self.check_permission(
            Permission::QueryAssets,
            org_id,
            Some(project_id.clone()),
        )?;
        
        // 2. 执行查询
        let assets = self.asset_service
            .query_assets(&project_id, asset_type.as_deref(), state.as_deref(), keyword.as_deref())
            .await?;
        
        Ok(CallToolResult::success(vec![Content::json(assets)?]))
    }

    /// 获取资产详情
    #[tool(name = "get_asset")]
    async fn get_asset(
        &self,
        #[tool(param(desc = "资产ID"))]
        asset_id: String,
    ) -> Result<CallToolResult, McpError> {
        let asset = self.asset_service
            .get_asset(&AssetId::parse(&asset_id)?)
            .await?;
        
        // 权限检查
        self.check_permission(
            Permission::AssetRead,
            asset.organization_id.clone(),
            asset.project_id.clone(),
        )?;
        
        Ok(CallToolResult::success(vec![Content::json(asset)?]))
    }

    /// 获取资产内容（通过 Resource）
    #[tool(name = "get_asset_content")]
    async fn get_asset_content(
        &self,
        #[tool(param(desc = "资产ID"))]
        asset_id: String,
    ) -> Result<CallToolResult, McpError> {
        let asset = self.asset_service
            .get_asset(&AssetId::parse(&asset_id)?)
            .await?;
        
        // 权限检查
        self.check_permission(
            Permission::AssetRead,
            asset.organization_id.clone(),
            asset.project_id.clone(),
        )?;
        
        // 通过 external_ref 获取实际内容
        let content = self.asset_service
            .fetch_content(&asset.external_ref)
            .await?;
        
        Ok(CallToolResult::success(vec![Content::text(content)]))
    }

    /// 刷新资产状态（重新检查 Dirty 状态）
    #[tool(name = "refresh_asset_state")]
    async fn refresh_asset_state(
        &self,
        #[tool(param(desc = "资产ID"))]
        asset_id: String,
    ) -> Result<CallToolResult, McpError> {
        let asset = self.asset_service
            .get_asset(&AssetId::parse(&asset_id)?)
            .await?;
        
        // 权限检查（需要状态刷新权限 - 可能修改 Dirty 队列）
        self.check_permission(
            Permission::StateRefresh,
            asset.organization_id.clone(),
            asset.project_id.clone(),
        )?;
        
        // 触发状态重新计算
        let new_state = self.asset_service
            .refresh_state(&AssetId::parse(&asset_id)?)
            .await?;
        
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Asset {} state refreshed. Current state: {:?}",
            asset_id, new_state
        ))]))
    }

    /// 获取依赖关系图
    #[tool(name = "get_dependency_graph")]
    async fn get_dependency_graph(
        &self,
        #[tool(param(desc = "资产ID"))]
        asset_id: String,
        #[tool(param(desc = "方向: upstream/downstream/all"))]
        direction: String,
        #[tool(param(desc = "查询深度", optional = true))]
        depth: Option<i32>,
    ) -> Result<CallToolResult, McpError> {
        let asset = self.asset_service
            .get_asset(&AssetId::parse(&asset_id)?)
            .await?;
        
        // 权限检查
        self.check_permission(
            Permission::QueryImpactAnalysis,
            asset.organization_id.clone(),
            asset.project_id.clone(),
        )?;
        
        let graph = self.impact_service
            .get_dependency_graph(&AssetId::parse(&asset_id)?, &direction, depth)
            .await?;
        
        Ok(CallToolResult::success(vec![Content::json(graph)?]))
    }

    /// 创建虚拟资产（查询上下文）
    #[tool(name = "create_virtual_asset")]
    async fn create_virtual_asset(
        &self,
        #[tool(param(desc = "目标资产类型"))]
        target_type: String,
        #[tool(param(desc = "锚点资产ID列表"))]
        anchor_ids: Vec<String>,
        #[tool(param(desc = "项目ID"))]
        project_id: String,
    ) -> Result<CallToolResult, McpError> {
        let org_id = self.organization_id();
        let project_id = ProjectId::parse(&project_id)?;
        
        // 权限检查
        self.check_permission(
            Permission::QueryVirtualContext,
            org_id,
            Some(project_id.clone()),
        )?;
        
        // 解析 anchor_ids，将错误转换为 McpError
        let anchor_asset_ids: Vec<AssetId> = anchor_ids
            .iter()
            .map(|s| AssetId::parse(s))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| McpError::invalid_params(format!("Invalid anchor asset ID: {}", e)))?;
        
        let virtual_asset = self.virtual_service
            .create_virtual(
                AssetTypeId::parse(&target_type)?,
                anchor_asset_ids,
                project_id,
                org_id, // 注入组织上下文
            )
            .await?;
        
        Ok(CallToolResult::success(vec![Content::json(virtual_asset)?]))
    }

    /// 获取虚拟资产依赖上下文
    #[tool(name = "get_virtual_context")]
    async fn get_virtual_context(
        &self,
        #[tool(param(desc = "虚拟实例ID"))]
        virtual_id: String,
    ) -> Result<CallToolResult, McpError> {
        let context = self.virtual_service
            .get_context(&Uuid::parse_str(&virtual_id)?)
            .await?;
        
        // 权限检查（虚拟实例继承创建者权限）
        if context.created_by != self.session_principal.id {
            self.check_permission(
                Permission::QueryVirtualContext,
                context.organization_id,
                Some(context.project_id),
            )?;
        }
        
        Ok(CallToolResult::success(vec![Content::json(context)?]))
    }

    /// 发布资产
    #[tool(name = "publish_asset")]
    async fn publish_asset(
        &self,
        #[tool(param(desc = "资产ID"))]
        asset_id: String,
        #[tool(param(desc = "发布说明"))]
        release_notes: String,
        #[tool(param(desc = "版本类型建议", optional = true))]
        suggested_type: Option<String>,
    ) -> Result<CallToolResult, McpError> {
        let asset = self.asset_service
            .get_asset(&AssetId::parse(&asset_id)?)
            .await?;
        
        // 权限检查
        self.check_permission(
            Permission::VersionPublish,
            asset.organization_id.clone(),
            asset.project_id.clone(),
        )?;
        
        // 注入发布者信息
        let result = self.version_service
            .publish_with_publisher(
                &AssetId::parse(&asset_id)?,
                &release_notes,
                suggested_type.as_deref(),
                &self.session_principal.id,
            )
            .await?;
        
        Ok(CallToolResult::success(vec![Content::json(result)?]))
    }

    /// 获取版本建议
    #[tool(name = "suggest_version")]
    async fn suggest_version(
        &self,
        #[tool(param(desc = "资产ID"))]
        asset_id: String,
    ) -> Result<CallToolResult, McpError> {
        let asset = self.asset_service
            .get_asset(&AssetId::parse(&asset_id)?)
            .await?;
        
        // 权限检查
        self.check_permission(
            Permission::VersionRead,
            asset.organization_id.clone(),
            asset.project_id.clone(),
        )?;
        
        let suggestion = self.version_service
            .suggest_version(&AssetId::parse(&asset_id)?)
            .await?;
        
        Ok(CallToolResult::success(vec![Content::json(suggestion)?]))
    }

    /// 手工 Clean 资产
    #[tool(name = "manual_clean_asset")]
    async fn manual_clean_asset(
        &self,
        #[tool(param(desc = "资产ID"))]
        asset_id: String,
        #[tool(param(desc = "处理的上游资产及版本"))]
        resolutions: Vec<DirtyResolutionInput>,
        #[tool(param(desc = "审查说明"))]
        comment: String,
    ) -> Result<CallToolResult, McpError> {
        let asset = self.asset_service
            .get_asset(&AssetId::parse(&asset_id)?)
            .await?;
        
        // 权限检查
        self.check_permission(
            Permission::StateManualClean,
            asset.organization_id.clone(),
            asset.project_id.clone(),
        )?;
        
        // 注入审核人信息
        let result = self.asset_service
            .manual_clean_with_reviewer(
                &AssetId::parse(&asset_id)?,
                resolutions,
                &comment,
                &self.session_principal.id,
            )
            .await?;
        
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Asset {} marked as clean by {}. Updated {} dependencies.",
            asset_id, self.session_principal.id, result.updated_count
        ))]))
    }
}

#[derive(Debug, Deserialize)]
pub struct DirtyResolutionInput {
    pub upstream_asset_id: String,
    pub from_version: String,
    pub to_version: String,
    pub review_result: String,
}

/// MCP Resources 定义
impl AdamMcpServer {
    pub fn resources(&self) -> Vec<Resource> {
        vec![
            Resource::new(
                "asset://{id}/content",
                "Asset content",
                Some("text/markdown"),
            ),
            Resource::new(
                "asset://{id}/metadata",
                "Asset metadata",
                Some("application/json"),
            ),
            Resource::new(
                "asset://{id}/dependencies",
                "Asset dependencies",
                Some("application/json"),
            ),
            Resource::new(
                "asset://{id}/versions",
                "Asset version history",
                Some("application/json"),
            ),
        ]
    }
    
    /// MCP Prompts 定义
    pub fn prompts(&self) -> Vec<Prompt> {
        vec![
            Prompt::new(
                "analyze_dirty_impact",
                "分析 Dirty 资产影响范围",
                Some("分析指定 Dirty 资产的上游变更对其下游资产的影响"),
            ),
            Prompt::new(
                "review_pending_changes",
                "审查待处理变更",
                Some("列出项目中所有 Dirty 资产并提供处理建议"),
            ),
            Prompt::new(
                "dependency_explainer",
                "依赖关系解释器",
                Some("解释资产的依赖关系链和变更传播路径"),
            ),
        ]
    }
}
```

---

## 8. 状态传播机制详解

### 8.1 状态转换规则

```rust
/// 状态转换矩阵
///
/// 当前状态 ╲ 事件    │ 上游发布     │ 自身发布    │ 手工 Clean  │ 归档
/// ───────────────────┼──────────────┼─────────────┼────────────┼────────
/// Clean             │ -> Dirty     │ -> Clean    │ -          │ -> Archived
/// Dirty             │ 合并来源     │ -> Clean    │ -> Clean   │ -> Archived
/// Archived          │ -            │ -           │ -          │ -
```

### 8.2 传播流程（含Dirty队列）

**设计说明**：Dirty 状态详情存储在 `dirty_queue` 表中，`asset_instances.current_state` 仅存储状态标签。

```
┌──────────────┐
│ 上游资产发布  │
│ 新版本 v1.1  │
└──────┬───────┘
       │
       ▼
┌──────────────────────────────────────────┐
│ 1. 查找直接下游资产（谁依赖我）          │
│    SELECT source_id FROM asset_dependencies│
│    WHERE target_id = 上游资产ID          │
└──────────────────────────────────────────┘
       │
       ▼
┌──────────────────────────────────────────┐
│ 2. 对每个下游资产                         │
│    a. 检查层级边界（同项目/同组织）       │
│    b. 插入 dirty_queue 条目               │
│       INSERT INTO dirty_queue            │
│       (asset_id, upstream_asset_id,       │
│        upstream_version, upstream_old_version,
│        impact_level, since)                │
│    c. 更新 asset_instances 状态           │
│       UPDATE SET current_state='dirty'   │
└──────────────────────────────────────────┘
       │
       ▼
┌──────────────────────────────────────────┐
│ 3. 发布 StateChanged 事件                 │
│    （不触发进一步的下游传播）             │
└──────────────────────────────────────────┘
```

### 8.3 手工 Clean 流程（含Dirty队列清理）

```
┌──────────────┐
│ 用户审查确认  │
└──────┬───────┘
       │
       ▼
┌──────────────────────────────────────────┐
│ 1. 更新 effective_version                 │
│    将当前有效依赖基线更新到上游最新版本   │
└──────────────────────────────────────────┘
       │
       ▼
┌──────────────────────────────────────────┐
│ 2. 标记 dirty_queue 已解决                │
│    UPDATE dirty_queue SET resolved=true    │
│    WHERE asset_id = ? AND upstream_asset_id = ?│
└──────────────────────────────────────────┘
       │
       ▼
┌──────────────────────────────────────────┐
│ 3. 检查是否还有未解决的 Dirty             │
│    SELECT COUNT(*) FROM dirty_queue      │
│    WHERE asset_id = ? AND resolved = false│
│    如果 count = 0，则恢复 Clean 状态      │
└──────────────────────────────────────────┘
       │
       ▼
┌──────────────────────────────────────────┐
│ 4. 记录 DirtyResolutionLog                │
│    审计日志，包含审查结论和说明           │
└──────────────────────────────────────────┘
       │
       ▼
┌──────────────────────────────────────────┐
│ 5. 状态恢复 Clean（如适用）               │
│    （不触发下游 Dirty）                   │
└──────────────────────────────────────────┘
```

---

## 9. 扩展性设计

### 9.1 资产类型插件系统

```rust
use async_trait::async_trait;

/// 资产类型扩展接口
trait AssetTypeExtension: Send + Sync {
    /// 资产类型名称
    fn type_name(&self) -> &str;
    
    /// 元数据 schema
    fn metadata_schema(&self) -> JsonSchema;
    
    /// 验证元数据
    fn validate_metadata(&self, metadata: &JsonValue) -> Result<(), ValidationError>;
    
    /// 内容获取器
    fn content_fetcher(&self) -> Box<dyn ContentFetcher>;
    
    /// 版本分析器（用于智能版本建议）
    fn version_analyzer(&self) -> Box<dyn VersionAnalyzer>;
}

/// 内容获取接口
#[async_trait]
trait ContentFetcher: Send + Sync {
    async fn fetch(&self, external_ref: &str) -> Result<AssetContent, FetchError>;
}

/// 版本分析接口
trait VersionAnalyzer: Send + Sync {
    fn analyze_changes(
        &self,
        previous: &AssetVersion,
        current_metadata: &JsonValue,
    ) -> ChangeAnalysis;
}

/// 插件注册表
pub struct AssetTypeRegistry {
    extensions: HashMap<String, Box<dyn AssetTypeExtension>>,
}

impl AssetTypeRegistry {
    pub fn register(&mut self, extension: Box<dyn AssetTypeExtension>) {
        self.extensions.insert(extension.type_name().to_string(), extension);
    }
    
    pub fn get(&self, type_name: &str) -> Option<&dyn AssetTypeExtension> {
        self.extensions.get(type_name).map(|b| b.as_ref())
    }
}
```

### 9.2 外部系统集成

```rust
use async_trait::async_trait;

/// 外部系统连接器接口
#[async_trait]
trait ExternalSystemConnector: Send + Sync {
    /// 系统类型
    fn system_type(&self) -> AssetSource;
    
    /// 验证引用是否有效
    async fn validate_ref(&self, external_ref: &str) -> Result<bool, ConnectorError>;
    
    /// 获取内容
    async fn fetch_content(&self, external_ref: &str) -> Result<AssetContent, ConnectorError>;
    
    /// 获取元数据
    async fn fetch_metadata(&self, external_ref: &str) -> Result<JsonValue, ConnectorError>;
}

/// Git 连接器实现
pub struct GitConnector {
    client: reqwest::Client,
    base_url: String,
    token: String,
}

impl ExternalSystemConnector for GitConnector {
    fn system_type(&self) -> AssetSource {
        AssetSource::Git
    }
    
    async fn fetch_content(&self, external_ref: &str) -> Result<AssetContent, ConnectorError> {
        // 解析 external_ref: "owner/repo/commit/sha"
        let parts: Vec<&str> = external_ref.split('/').collect();
        if parts.len() != 4 {
            return Err(ConnectorError::InvalidReference);
        }
        
        let url = format!(
            "{}/repos/{}/{}/commits/{}",
            self.base_url, parts[0], parts[1], parts[3]
        );
        
        let response = self.client
            .get(&url)
            .header("Authorization", format!("token {}", self.token))
            .send()
            .await?;
            
        let commit: GitCommit = response.json().await?;
        
        Ok(AssetContent {
            content: commit.message,
            format: ContentFormat::Markdown,
        })
    }
    // ...
}
```

---

## 10. 性能优化策略

### 10.1 数据库优化

```sql
-- 依赖图查询优化（递归 CTE）
-- 注意：source_id = 下游(依赖方), target_id = 上游(被依赖方)
WITH RECURSIVE dependency_tree AS (
    -- 起始节点：从下游资产开始
    SELECT target_id AS asset_id, 1 AS depth
    FROM asset_dependencies
    WHERE source_id = $1
    
    UNION ALL
    
    -- 递归查找上游（被依赖方）
    SELECT d.target_id, dt.depth + 1
    FROM asset_dependencies d
    JOIN dependency_tree dt ON d.source_id = dt.asset_id
    WHERE dt.depth < $2  -- 限制深度
)
SELECT DISTINCT asset_id FROM dependency_tree;

-- 索引策略
CREATE INDEX CONCURRENTLY idx_deps_source_target ON asset_dependencies(source_id, target_id);
CREATE INDEX CONCURRENTLY idx_asset_state_updated ON asset_instances(current_state, updated_at);
```

### 10.2 缓存策略

```rust
use redis::AsyncCommands;

pub struct CachedAssetRepository {
    inner: Box<dyn AssetRepository>,
    redis: redis::aio::MultiplexedConnection,
    ttl: Duration,
}

impl AssetRepository for CachedAssetRepository {
    async fn find_by_id(&self, id: &AssetId) -> Result<Option<AssetInstance>, RepositoryError> {
        let cache_key = format!("asset:{}", id);
        
        // 尝试从缓存获取
        let mut conn = self.redis.clone();
        if let Ok(cached) = conn.get::<_, String>(&cache_key).await {
            if let Ok(asset) = serde_json::from_str::<AssetInstance>(&cached) {
                return Ok(Some(asset));
            }
        }
        
        // 从数据库获取
        let asset = self.inner.find_by_id(id).await?;
        
        // 写入缓存
        if let Some(ref a) = asset {
            match serde_json::to_string(a) {
                Ok(serialized) => {
                    if let Err(err) = conn.set_ex::<_, _, ()>(&cache_key, serialized, self.ttl.as_secs()).await {
                        tracing::warn!(?err, "failed to write asset cache");
                    }
                }
                Err(err) => {
                    tracing::warn!(?err, "failed to serialize asset for cache");
                }
            }
        }
        
        Ok(asset)
    }
    
    async fn save(&self, asset: &AssetInstance) -> Result<(), RepositoryError> {
        // 先写入数据库
        self.inner.save(asset).await?;
        
        // 删除缓存（下次读取时重建）
        let cache_key = format!("asset:{}", asset.id);
        let mut conn = self.redis.clone();
        if let Err(err) = conn.del::<_, ()>(&cache_key).await {
            tracing::warn!(?err, "failed to invalidate asset cache");
        }
        
        Ok(())
    }
}
```

### 10.3 DAG 缓存

```rust
/// DAG 缓存服务
pub struct DagCache {
    graph: Arc<RwLock<DiGraph<AssetId, ()>>>,
    node_map: Arc<RwLock<HashMap<AssetId, NodeIndex>>>,
}

impl DagCache {
    /// 增量更新依赖图
    pub async fn add_edge(&self, source: AssetId, target: AssetId) {
        let mut graph = self.graph.write().await;
        let mut map = self.node_map.write().await;
        
        let source_idx = *map.entry(source).or_insert_with(|| graph.add_node(source));
        let target_idx = *map.entry(target).or_insert_with(|| graph.add_node(target));
        
        graph.add_edge(source_idx, target_idx, ());
    }
    
    /// 内存中进行环检测（比数据库查询快）
    pub async fn would_create_cycle(&self, source: AssetId, target: AssetId) -> bool {
        // 使用 petgraph 的环检测算法
        let graph = self.graph.read().await;
        let map = self.node_map.read().await;
        
        // 临时添加边并检测
        if let (Some(&s), Some(&t)) = (map.get(&source), map.get(&target)) {
            // 检查是否存在从 target 到 source 的路径
            petgraph::algo::has_path_connecting(&*graph, t, s, None)
        } else {
            false
        }
    }
}
```

---

## 11. 部署架构

### 11.1 服务拆分

```
┌─────────────────────────────────────────────────────────────┐
│                        Load Balancer                        │
└─────────────────────────────────────────────────────────────┘
                              │
           ┌──────────────────┼──────────────────┐
           ▼                  ▼                  ▼
┌──────────────────┐ ┌──────────────────┐ ┌──────────────────┐
│   API Server 1   │ │   API Server 2   │ │   API Server N   │
│   (REST API)     │ │                  │ │                  │
└────────┬─────────┘ └────────┬─────────┘ └────────┬─────────┘
         │                    │                    │
         └────────────────────┼────────────────────┘
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                   PostgreSQL Cluster                        │
│              (Primary + Read Replicas)                      │
└─────────────────────────────────────────────────────────────┘
                              │
┌──────────────────┐ ┌──────────────────┐ ┌──────────────────┐
│   Redis Cluster  │ │   MCP Server     │ │   Worker Pool  │
│   (Cache + MQ)   │ │   (Dedicated)    │ │   (Async Jobs) │
└──────────────────┘ └──────────────────┘ └──────────────────┘
```

### 11.2 配置管理

```rust
use config::{Config, ConfigError, Environment, File};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub redis: RedisConfig,
    pub mcp: McpConfig,
}

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub workers: usize,
}

#[derive(Debug, Deserialize)]
pub struct DatabaseConfig {
    pub url: String,
    pub max_connections: u32,
    pub min_connections: u32,
}

impl AppConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        let builder = Config::builder()
            .add_source(File::with_name("config/default").required(false))
            .add_source(File::with_name("config/local").required(false))
            .add_source(Environment::with_prefix("ADAM").separator("__"))
            .build()?;
            
        builder.try_deserialize()
    }
}
```

---

## 12. 测试策略

### 12.1 单元测试

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_state_transitions() {
        // AssetState 现在是简化枚举，仅包含状态标签
        let clean = AssetState::Clean;
        let dirty = AssetState::Dirty;
        let archived = AssetState::Archived;
        
        // 验证状态值
        assert_eq!(clean, AssetState::Clean);
        assert_eq!(dirty, AssetState::Dirty);
        assert_eq!(archived, AssetState::Archived);
        
        // 验证状态转换约束
        assert!(clean.can_publish());
        assert!(dirty.can_publish());
        assert!(!archived.can_publish()); // 归档状态不能发布
    }

    #[test]
    fn dependency_boundary_validation() {
        let org_a = OrganizationId(Uuid::new_v4());
        let org_b = OrganizationId(Uuid::new_v4());
        let proj_a = ProjectId(Uuid::new_v4());
        let proj_b = ProjectId(Uuid::new_v4());
        
        // 同组织、项目级依赖项目级 - 应该通过
        let same_project = DependencyBoundaryContext {
            source_level: AssetLevel::Project,
            source_project_id: Some(proj_a),
            source_org_id: org_a,
            target_level: AssetLevel::Project,
            target_project_id: Some(proj_a),
            target_org_id: org_a,
        };
        assert!(same_project.validate().is_ok());
        
        // 跨项目 - 应该失败
        let cross_project = DependencyBoundaryContext {
            source_level: AssetLevel::Project,
            source_project_id: Some(proj_a),
            source_org_id: org_a,
            target_level: AssetLevel::Project,
            target_project_id: Some(proj_b), // 不同项目
            target_org_id: org_a,
        };
        assert!(cross_project.validate().is_err());
        
        // 跨组织 - 应该失败
        let cross_org = DependencyBoundaryContext {
            source_level: AssetLevel::Project,
            source_project_id: Some(proj_a),
            source_org_id: org_a,
            target_level: AssetLevel::Organization,
            target_project_id: None,
            target_org_id: org_b, // 不同组织
        };
        assert!(cross_org.validate().is_err());
    }

    #[test]
    fn dag_cycle_detection() {
        let existing = vec![
            (AssetId(Uuid::new_v4()), AssetId(Uuid::new_v4())),
        ];
        
        // 自循环
        let self_loop = (existing[0].0, existing[0].0);
        assert!(DagValidator::validate_no_cycle(&existing, self_loop).is_err());
        
        // 间接循环
        let a = AssetId(Uuid::new_v4());
        let b = AssetId(Uuid::new_v4());
        let c = AssetId(Uuid::new_v4());
        
        let edges = vec![(a, b), (b, c)];
        let cycle_edge = (c, a); // 会创建 A -> B -> C -> A
        assert!(DagValidator::validate_no_cycle(&edges, cycle_edge).is_err());
    }

    #[test]
    fn version_identifier() {
        // SemVer
        let semver = SemVersion::parse("1.2.3").unwrap();
        let version_id = VersionIdentifier::from_semver(&semver);
        assert_eq!(version_id.as_str(), "1.2.3");
        
        // ExternalRef (commit SHA)
        let commit_sha = "abc123def456";
        let external_ref = VersionIdentifier::from_external_ref(commit_sha);
        assert_eq!(external_ref.as_str(), "abc123def456");
        
        // Composite
        let composite = VersionIdentifier::from_composite("v1.0", 5);
        assert_eq!(composite.as_str(), "v1.0.005");
        
        // SemVer suggestion
        let v1 = SemVersion::parse("1.2.3").unwrap();
        assert_eq!(v1.suggest_next(ChangeType::Major).to_string(), "2.0.0");
        assert_eq!(v1.suggest_next(ChangeType::Minor).to_string(), "1.3.0");
        assert_eq!(v1.suggest_next(ChangeType::Patch).to_string(), "1.2.4");
    }
}
```

### 12.2 集成测试

```rust
#[cfg(test)]
mod integration_tests {
    use super::*;
    use sqlx::PgPool;

    #[sqlx::test]
    async fn create_and_publish_asset(pool: PgPool) {
        // 设置
        let repo = PostgresAssetRepository::new(pool);
        let service = AssetServiceImpl::new(Box::new(repo));
        
        // 创建组织
        let org_id = OrganizationId(Uuid::new_v4());
        
        // 创建项目
        let proj_id = ProjectId(Uuid::new_v4());
        
        // 创建资产（应用服务命令由 API 层注入 organization_id 和 publisher）
        let asset = service.create(CreateAssetCommand {
            name: "Test Asset".to_string(),
            type_id: AssetTypeId(Uuid::new_v4()),
            external_ref: "test/123".to_string(),
            source: AssetSource::Git,
            level: AssetLevel::Project,
            project_id: Some(proj_id),
            organization_id: org_id,
            metadata: json!({}),
            dependencies: vec![],
            release_notes: None,
            initial_version: Some("0.1.0".to_string()),
            publisher: "user1".to_string(),
        }).await.unwrap();
        
        assert_eq!(asset.current_state, AssetState::Clean);
        assert_eq!(asset.current_version.as_str(), "0.1.0");
        assert_eq!(asset.organization_id, org_id);
        
        // 验证幂等键生成
        assert!(!asset.idempotency_key.is_empty());
    }

    #[sqlx::test]
    async fn idempotency_key_prevents_duplicate(pool: PgPool) {
        // 测试自动化注册的幂等性
        let repo = PostgresAssetRepository::new(pool);
        
        let org_id = OrganizationId(Uuid::new_v4());
        let proj_id = ProjectId(Uuid::new_v4());
        
        // 首次创建
        let asset1 = AssetInstance::new(
            AssetTypeId(Uuid::new_v4()),
            "Test".to_string(),
            "git/repo/abc123".to_string(),
            AssetSource::Git,
            AssetLevel::Project,
            org_id,
            Some(proj_id),
            VersionIdentifier::from_external_ref("abc123"),
            json!({}),
            "user1".to_string(),
            vec![],
        ).unwrap();
        
        repo.save(&asset1).await.unwrap();
        
        // 尝试创建相同幂等键的资产应该失败
        let asset2 = AssetInstance::new(
            AssetTypeId(Uuid::new_v4()),
            "Test2".to_string(),  // 不同名称
            "git/repo/abc123".to_string(),  // 相同 external_ref
            AssetSource::Git,
            AssetLevel::Project,
            org_id,
            Some(proj_id),
            VersionIdentifier::from_external_ref("abc123"),
            json!({}),
            "user1".to_string(),
            vec![],
        ).unwrap();
        
        // 应该由于幂等键冲突而失败
        assert!(repo.save(&asset2).await.is_err());
    }

    #[sqlx::test]
    async fn state_propagation(pool: PgPool) {
        // 测试状态传播
        let repo = PostgresAssetRepository::new(pool);
        let dirty_queue_repo = PostgresDirtyQueueRepository::new(pool);
        let propagator = StatePropagator::new(Box::new(MockEventPublisher));
        
        let org_id = OrganizationId(Uuid::new_v4());
        let proj_id = ProjectId(Uuid::new_v4());
        
        // 创建上游资产
        let upstream = create_test_asset_with_org(&repo, "Upstream", org_id, Some(proj_id)).await;
        
        // 创建下游资产
        let downstream = create_test_asset_with_dep(&repo, "Downstream", org_id, Some(proj_id), upstream.id).await;
        
        // 发布上游新版本
        propagator.on_asset_published(
            upstream.id,
            VersionIdentifier::from_semver(&SemVersion::parse("1.1.0").unwrap()),
            &repo,
            &dirty_queue_repo,
        ).await.unwrap();
        
        // 验证下游变为 Dirty
        let updated = repo.find_by_id(&downstream.id).await.unwrap().unwrap();
        assert_eq!(updated.current_state, AssetState::Dirty);
        
        // 验证 dirty_queue 条目
        let dirty_entries = dirty_queue_repo.find_by_asset(&downstream.id).await.unwrap();
        assert_eq!(dirty_entries.len(), 1);
        assert_eq!(dirty_entries[0].upstream_asset_id, upstream.id);
    }

    #[sqlx::test]
    async fn dependency_boundary_validation(pool: PgPool) {
        // 测试依赖边界验证
        let repo = PostgresAssetRepository::new(pool);
        
        let org_a = OrganizationId(Uuid::new_v4());
        let org_b = OrganizationId(Uuid::new_v4());
        let proj_a1 = ProjectId(Uuid::new_v4());
        let proj_a2 = ProjectId(Uuid::new_v4());
        
        // 组织 A 项目 1 的资产
        let asset_a1 = create_test_asset_with_org(&repo, "Asset A1", org_a, Some(proj_a1)).await;
        
        // 组织 A 项目 2 的资产
        let asset_a2 = create_test_asset_with_org(&repo, "Asset A2", org_a, Some(proj_a2)).await;
        
        // 组织 B 的资产
        let asset_b = create_test_asset_with_org(&repo, "Asset B", org_b, None).await;
        
        // 尝试创建跨项目依赖（应该失败）
        let result = repo.create_dependency(AssetDependency {
            id: Uuid::new_v4(),
            source_id: asset_a1.id,  // 项目1
            target_id: asset_a2.id,  // 项目2
            relationship: DependencyRelation::DependsOn,
            declared_version: VersionIdentifier::from_external_ref("v1.0"),
            effective_version: VersionIdentifier::from_external_ref("v1.0"),
            effective_updated_by: "test".to_string(),
            effective_updated_at: Utc::now(),
            effective_reason: EffectiveUpdateReason::Publish,
            created_at: Utc::now(),
        }).await;
        
        assert!(result.is_err()); // 应该由于跨项目边界而失败
        
        // 尝试创建跨组织依赖（应该失败）
        let result = repo.create_dependency(AssetDependency {
            id: Uuid::new_v4(),
            source_id: asset_a1.id,
            target_id: asset_b.id,  // 组织B
            relationship: DependencyRelation::DependsOn,
            declared_version: VersionIdentifier::from_external_ref("v1.0"),
            effective_version: VersionIdentifier::from_external_ref("v1.0"),
            effective_updated_by: "test".to_string(),
            effective_updated_at: Utc::now(),
            effective_reason: EffectiveUpdateReason::Publish,
            created_at: Utc::now(),
        }).await;
        
        assert!(result.is_err()); // 应该由于跨组织边界而失败
    }
}
```

---

## 13. 总结

### 13.1 关键设计决策

| 决策 | 选择 | 理由 |
|------|------|------|
| 架构风格 | 六边形架构 | 领域逻辑独立，易于测试和演进 |
| 语言 | Rust | 内存安全、高性能、优秀的并发模型 |
| Web 框架 | Axum | 基于 Tower，生态成熟，支持 async |
| 数据库 | PostgreSQL | 支持复杂查询、JSON、递归 CTE |
| ORM | SQLx | 编译时检查 SQL，无运行时开销 |
| DAG 库 | petgraph | 成熟的图算法库 |
| MCP | rmcp | Rust MCP SDK |
| 状态存储 | 分离存储：current_state + dirty_queue 表 | Dirty详情持久化，支持优先级排序和审计 |
| 版本标识 | VersionStrategy（SemVer/ExternalRef/Composite） | 适应不同资产类型的版本需求 |
| 幂等键 | 标准化格式：source:org:project?:ref | 支持自动化注册和重复检测 |

### 13.2 架构不变量（关键约束）

| 不变量 | 定义 |
|--------|------|
| 依赖方向 | source_id = 下游，target_id = 上游；边方向：下游 -> 上游 |
| 层级边界 | 项目级只能依赖同项目内的项目级；组织级只能依赖同组织内的组织级；项目级与组织级之间不建立持久化依赖边 |
| 组织边界 | 依赖不得跨组织；查询返回组织级资产但不参与 Dirty 传播 |
| 状态传播 | 仅发布事件触发下游 Dirty；状态变更不传播 |
| 幂等保证 | 自动化注册使用标准化 idempotency_key |

### 13.3 实现优先级

**第一阶段（MVP）**：
1. 组织/项目基础数据模型
2. 资产类型管理（FR-001, FR-002）
3. 资产实例 CRUD（FR-005, FR-006）
4. 依赖关系建立与验证（FR-003, FR-009, BR-002）
5. 版本发布管理（FR-012）
6. Dirty 队列和基本状态管理（FR-015, FR-016）
7. 状态传播（FR-017）

**第二阶段**：
1. MCP Server 接口（FR-023）
2. 虚拟实例（FR-022）
3. 影响分析（FR-021）
4. 手工 Clean（FR-016）
5. 审计日志（FR-025）

**第三阶段**：
1. Git Hooks 集成（FR-019A）
2. CI/CD 集成（FR-019B）
3. 保留策略（FR-027）
4. Skill 场景封装（FR-024）

---

*文档结束*
