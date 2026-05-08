# ADAM 实施计划（TDD 版本）

**版本**: 1.1  
**日期**: 2026-05-08  
**状态**: TDD 草案  
**测试目标**: 单元测试覆盖率 > 80%

---

## TDD 核心原则

```
┌─────────┐     ┌─────────┐     ┌─────────┐
│  RED    │ --> │  GREEN  │ --> │ REFACTOR│
│ 写测试   │     │ 写实现   │     │ 重构    │
│ 测试失败 │     │ 测试通过 │     │ 保持通过 │
└─────────┘     └─────────┘     └────┬────┘
     ↑                                 │
     └─────────────────────────────────┘
```

**强制要求**:
1. 任何功能代码提交前必须有对应的测试
2. 测试先写，实现后写
3. 重构必须在测试保护下进行
4. 集成测试使用 sqlx-test，同样需要遵循 RED-GREEN-REFACTOR

---

## Phase 0: 项目脚手架（Week 0）

### Week 0: Workspace / Crate 结构

#### 任务 0.1: Cargo Workspace 初始化

**RED**: 创建项目结构，配置依赖，确保 `cargo build` 成功

```bash
# 在仓库根创建顶层目录（当前目录已经是 ADAM）
mkdir -p crates
mkdir -p tests/{integration,e2e}
mkdir -p migrations
mkdir -p docs

# 创建虚拟 workspace（不是普通 package）
# 手动创建 Cargo.toml，不用 cargo init
cat > Cargo.toml << 'EOF'
[workspace]
resolver = "2"
members = [
    "crates/adam-domain",
    "crates/adam-application",
    "crates/adam-infrastructure",
    "crates/adam-adapters",
    "crates/adam-server",
]

[workspace.package]
edition = "2024"
rust-version = "1.85"

[workspace.dependencies]
# 见下方完整配置
EOF

# 逐个创建 crate；由 cargo new 创建目标目录，避免对已存在目录执行 cargo new
cargo new --lib crates/adam-domain
cargo new --lib crates/adam-application
cargo new --lib crates/adam-infrastructure
cargo new --lib crates/adam-adapters
cargo new --bin crates/adam-server

# 验证
cargo check  # 应该成功（空 crate）
```

**GREEN**: 完成 workspace 配置

> ⚠️ **注意**: 以下依赖版本与 `architecture.md` 5.1 节保持一致，作为唯一依赖来源。

```toml
# Cargo.toml (workspace root)
[workspace]
resolver = "2"
members = [
    "crates/adam-domain",
    "crates/adam-application", 
    "crates/adam-infrastructure",
    "crates/adam-adapters",
    "crates/adam-server",
]

[workspace.dependencies]
# 异步运行时
async-trait = "0.1"
tokio = { version = "1.44", features = ["full"] }
tokio-util = "0.7"

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

# MCP
rmcp = "0.3"

# 验证
validator = { version = "0.20", features = ["derive"] }

# UUID
uuid = { version = "1.16", features = ["v4", "serde"] }

# 时间
chrono = { version = "0.4", features = ["serde"] }

# 图算法
petgraph = "0.7"

# 版本解析
semver = "1.0"

# 错误处理
thiserror = "2.0"
anyhow = "1.0"

# 日志/追踪
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

# 配置
config = "0.15"

# CLI
clap = { version = "4.5", features = ["derive"] }

# JSON Schema
schemars = "0.8"
jsonschema = "0.29"

# HTTP 客户端
reqwest = { version = "0.12", features = ["json"] }

# 缓存（可选，非 MVP）
redis = { version = "0.29", features = ["tokio-comp"], optional = true }

# 测试
mockall = "0.13"
tokio-test = "0.4"
```

**验收标准**:
- [ ] `cargo check` 在 workspace root 成功
- [ ] 每个 crate 有基础 `Cargo.toml` 和 `src/lib.rs`
- [ ] CI 配置完成（GitHub Actions）

#### 任务 0.2: Crate 结构定义

**GREEN**: 各 crate 基础配置

```toml
# crates/adam-domain/Cargo.toml
[package]
name = "adam-domain"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true

[dependencies]
serde = { workspace = true }
chrono = { workspace = true }
uuid = { workspace = true }
thiserror = { workspace = true }
async-trait = { workspace = true }

[dev-dependencies]
tokio-test = { workspace = true }
```

```toml
# crates/adam-infrastructure/Cargo.toml
[package]
name = "adam-infrastructure"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true

[dependencies]
adam-domain = { path = "../adam-domain" }
sqlx = { workspace = true }
tokio = { workspace = true }
uuid = { workspace = true }
chrono = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
redis = { workspace = true, optional = true }

[features]
default = []
cache = ["dep:redis"]
```

**验收标准**:
- [ ] 所有 crate 依赖关系正确（无循环依赖）
- [ ] `cargo tree` 检查通过
- [ ] 各 crate `src/lib.rs` 可编译

#### 任务 0.3: 测试框架配置

**GREEN**: 配置 tarpaulin、sqlx-test

```toml
# .cargo/config.toml
[env]
DATABASE_URL = "postgres://postgres:postgres@localhost/adam_test"

[build]
target-dir = "target"
```

```yaml
# .github/workflows/ci.yml
name: CI

on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest
    services:
      postgres:
        image: postgres:15
        env:
          POSTGRES_PASSWORD: postgres
        options: >-
          --health-cmd pg_isready
          --health-interval 10s
          --health-timeout 5s
          --health-retries 5
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - name: Install sqlx-cli
        run: cargo install sqlx-cli --no-default-features --features native,postgres
      - name: Setup database
        run: |
          sqlx database create
          sqlx migrate run
      - name: Test
        run: cargo test --workspace --all-features
      - name: Coverage
        uses: actions-rs/tarpaulin@v0.1
        with:
          args: '--fail-under 80'
```

**验收标准**:
- [ ] `cargo test --workspace` 可以运行（空测试通过）
- [ ] `cargo tarpaulin` 可生成报告
- [ ] CI pipeline 可执行

---

## 第一阶段：领域层（Week 1-2）

### Week 1: 核心领域模型

#### 任务 1.1: AssetState 状态机

**TDD 循环**:

```rust
// RED: 先写测试 - tests/asset_state_test.rs
#[test]
fn asset_state_can_transition_from_clean_to_dirty() {
    let state = AssetState::Clean;
    assert!(state.can_transition_to(AssetState::Dirty));
}

#[test]
fn asset_state_cannot_transition_from_archived_to_any() {
    let state = AssetState::Archived;
    assert!(!state.can_transition_to(AssetState::Clean));
    assert!(!state.can_transition_to(AssetState::Dirty));
}

#[test]
fn asset_state_is_dirty_returns_true_for_dirty() {
    assert!(AssetState::Dirty.is_dirty());
    assert!(!AssetState::Clean.is_dirty());
}
```

**实现步骤**:
1. [x] RED: 创建 `tests/asset_state_test.rs`，定义测试（编译失败）
2. [ ] GREEN: 实现 `AssetState` enum 和 `can_transition_to()` 方法
3. [ ] REFACTOR: 优化状态转换表实现

**验收标准**:
- [ ] `cargo test asset_state` 全部通过
- [ ] 覆盖率 100%

#### 任务 1.2: DependencyBoundaryContext

**TDD 循环**:

```rust
// RED: 测试边界规则
#[test]
fn project_asset_cannot_depend_on_organization_asset() {
    let ctx = DependencyBoundaryContext {
        source_level: AssetLevel::Project,
        target_level: AssetLevel::Organization,
        source_project_id: Some(ProjectId::new()),
        target_project_id: None,
        // ...
    };
    assert!(matches!(ctx.validate(), Err(DependencyError::ProjectCannotDependOnOrganization)));
}

#[test]
fn same_project_dependency_is_valid() {
    let project_id = ProjectId::new();
    let ctx = DependencyBoundaryContext {
        source_level: AssetLevel::Project,
        target_level: AssetLevel::Project,
        source_project_id: Some(project_id),
        target_project_id: Some(project_id),
        // ...
    };
    assert!(ctx.validate().is_ok());
}
```

**实现步骤**:
1. [ ] RED: 写边界验证测试（编译失败）
2. [ ] GREEN: 实现 `DependencyBoundaryContext::validate()`
3. [ ] REFACTOR: 提取验证规则为独立函数

**验收标准**:
- [ ] 4 条边界规则测试通过
- [ ] 跨组织/跨项目/层级边界测试覆盖

#### 任务 1.3: DAGValidator

**TDD 循环**:

```rust
// RED: 测试循环检测
#[test]
fn detects_simple_cycle() {
    let edges = vec![("A", "B"), ("B", "C"), ("C", "A")];
    let result = DAGValidator::validate_no_cycle(&edges);
    assert!(matches!(result, Err(DAGError::CycleDetected(_))));
}

#[test]
fn valid_dag_passes() {
    let edges = vec![("A", "B"), ("A", "C"), ("B", "D")];
    assert!(DAGValidator::validate_no_cycle(&edges).is_ok());
}
```

**验收标准**:
- [ ] 循环检测测试通过
- [ ] 复杂图结构测试通过

---

### Week 2: 仓储接口 + 内存实现

#### 任务 2.1: 定义仓储 Trait

**TDD 循环**:

```rust
// RED: 先定义行为测试
#[async_trait]
trait AssetRepository {
    async fn create(&self, cmd: &CreateAssetCommand) -> Result<AssetInstance, RepositoryError>;
    async fn find_by_id(&self, id: &AssetId) -> Result<Option<AssetInstance>, RepositoryError>;
    async fn update_state(&self, id: &AssetId, state: AssetState) -> Result<(), RepositoryError>;
}

// 先写内存实现测试
#[tokio::test]
async fn memory_repo_creates_asset() {
    let repo = InMemoryAssetRepository::new();
    let cmd = CreateAssetCommand { /* ... */ };
    let asset = repo.create(&cmd).await.unwrap();
    assert_eq!(asset.name, cmd.name);
}
```

**验收标准**:
- [ ] AssetRepository trait 定义完成
- [ ] InMemoryAssetRepository 实现 + 测试通过
- [ ] DirtyQueueRepository trait + InMemory 实现 + 测试通过

#### 任务 2.2: StatePropagator

**TDD 循环**:

```rust
// RED: 测试传播逻辑
#[tokio::test]
async fn propagate_dirty_creates_dirty_queue_entries() {
    // Setup: B -> A, C -> A（source=下游依赖方，target=上游被依赖方；B 和 C 依赖 A）
    let repo = InMemoryAssetRepository::with_data(vec![a, b, c]);
    let dirty_repo = InMemoryDirtyQueueRepository::new();
    let propagator = StatePropagator::new();
    
    // 当 A 发布新版本
    let affected = propagator.on_asset_published(a.id, version_2, &repo, &dirty_repo).await.unwrap();
    
    // Assert: B 和 C 都有 DirtyQueueEntry
    assert_eq!(affected.len(), 2);
    let b_dirty = dirty_repo.find_unresolved_by_asset(&b.id).await.unwrap();
    assert_eq!(b_dirty.len(), 1);
    assert_eq!(b_dirty[0].upstream_asset_id, a.id);
}

#[tokio::test]
async fn propagate_dirty_updates_existing_dirty_entry() {
    // 如果已有 Dirty 条目，更新为新版本
}

// P1 FIX: 增加 Archived 终态测试
#[tokio::test]
async fn archived_downstream_is_skipped() {
    // Setup: A -> B(Archived), A -> C(Clean)
    let archived_b = AssetInstance {
        current_state: AssetState::Archived,
        // ...
    };
    let repo = InMemoryAssetRepository::with_data(vec![a, archived_b, c]);
    let dirty_repo = InMemoryDirtyQueueRepository::new();
    let propagator = StatePropagator::new();
    
    // 当 A 发布新版本
    let affected = propagator.on_asset_published(a.id, version_2, &repo, &dirty_repo).await.unwrap();
    
    // Assert: 只有 C 被标记 Dirty，B(Archived) 被跳过
    assert_eq!(affected.len(), 1);
    assert!(dirty_repo.find_unresolved_by_asset(&archived_b.id).await.unwrap().is_empty());
    assert_eq!(dirty_repo.find_unresolved_by_asset(&c.id).await.unwrap().len(), 1);
}

#[tokio::test]
async fn archived_upstream_does_not_trigger_dirty() {
    // Archived 资产发布版本不应该触发下游 Dirty
    let archived_a = AssetInstance {
        current_state: AssetState::Archived,
        // ...
    };
    let repo = InMemoryAssetRepository::with_data(vec![archived_a, b, c]);
    let dirty_repo = InMemoryDirtyQueueRepository::new();
    let propagator = StatePropagator::new();

    // 当 Archived A 发布新版本（理论上不应发生，但测试防护）
    let result = propagator.on_asset_published(archived_a.id, version_2, &repo, &dirty_repo).await;

    // Assert: 应返回错误或被忽略，不写入 dirty_queue
    assert!(result.is_err() || result.unwrap().is_empty());
    assert!(dirty_repo.find_unresolved_by_asset(&b.id).await.unwrap().is_empty());
    assert!(dirty_repo.find_unresolved_by_asset(&c.id).await.unwrap().is_empty());
}
```

**验收标准**:
- [ ] 传播测试通过（下游标记 Dirty）
- [ ] 重复 Dirty 处理测试通过
- [ ] 边界检查测试通过
- [ ] **Archived 下游被跳过测试通过**
- [ ] **Archived 上游不触发传播测试通过**

---

## 第二阶段：应用层 + 数据库实现（Week 3-4）

### Week 3: PostgreSQL 实现

#### 任务 3.1: 数据库迁移（先写迁移测试）

**TDD 循环**:

```rust
// RED: 先写迁移正确性测试（在 sqlx-test 中）
#[sqlx::test]
async fn migration_creates_tables(pool: PgPool) {
    // 验证表结构
    let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM asset_instances")
        .fetch_one(&pool)
        .await
        .unwrap();
    // 应该可以查询（表存在）
}

#[sqlx::test]
async fn partial_unique_index_on_dirty_queue(pool: PgPool) {
    // 测试部分唯一索引约束
    let id1 = Uuid::new_v4();
    let id2 = Uuid::new_v4();
    
    // 插入第一条未解决 Dirty
    sqlx::query("INSERT INTO dirty_queue (id, asset_id, upstream_asset_id, ...) VALUES ...")
        .execute(&pool)
        .await
        .unwrap();
    
    // 插入相同 asset/upstream 组合应该失败
    let result = sqlx::query("INSERT INTO ...").execute(&pool).await;
    assert!(result.is_err());
}

// P1 FIX: 增加核心边界触发器测试（错误消息与 architecture.md SQL 触发器保持一致）
#[sqlx::test]
async fn trigger_rejects_project_to_organization_dependency(pool: PgPool) {
    // Setup: 创建组织和资产
    let org_id = Uuid::new_v4();
    let project_asset = Uuid::new_v4();
    let org_asset = Uuid::new_v4();

    // 尝试创建 Project -> Organization 依赖
    let result = sqlx::query(
        "INSERT INTO asset_dependencies (id, source_id, target_id, ...) VALUES ..."
    )
    .bind(project_asset) // source: Project
    .bind(org_asset)     // target: Organization
    .execute(&pool)
    .await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    // 与 SQL 触发器错误消息一致；规则编号由测试名/注释说明，避免和数据库文案漂移
    assert!(err.to_string().contains("Project-level asset cannot persist dependency on organization-level asset"));
}

#[sqlx::test]
async fn trigger_rejects_organization_to_project_dependency(pool: PgPool) {
    // Setup: 创建组织和资产
    let org_id = Uuid::new_v4();
    let org_asset = Uuid::new_v4();
    let project_asset = Uuid::new_v4();

    // 尝试创建 Organization -> Project 依赖
    let result = sqlx::query(
        "INSERT INTO asset_dependencies (id, source_id, target_id, ...) VALUES ..."
    )
    .bind(org_asset)      // source: Organization
    .bind(project_asset)  // target: Project
    .execute(&pool)
    .await;

    assert!(result.is_err());
    // 与 SQL 触发器错误消息一致
    assert!(result.unwrap_err().to_string().contains("Organization-level asset can only depend on organization-level assets"));
}

#[sqlx::test]
async fn trigger_rejects_cross_project_dependency(pool: PgPool) {
    // Setup: 两个不同项目的项目级资产
    let project_a = Uuid::new_v4();
    let project_b = Uuid::new_v4();
    let asset_in_a = Uuid::new_v4();
    let asset_in_b = Uuid::new_v4();

    // 尝试创建跨项目依赖
    let result = sqlx::query("INSERT INTO asset_dependencies ...").execute(&pool).await;
    assert!(result.is_err());
    // 与 SQL 触发器错误消息一致；跨项目依赖属于 BR-008 边界约束
    assert!(result.unwrap_err().to_string().contains("Cross-project dependency not allowed"));
}

#[sqlx::test]
async fn trigger_rejects_cross_organization_dependency(pool: PgPool) {
    // Setup: 两个不同组织的资产
    // 尝试创建跨组织依赖
}

#[sqlx::test]
async fn trigger_rejects_asset_type_org_mismatch(pool: PgPool) {
    // 测试 asset_instances.type_id 必须与 organization_id 同组织
}

#[sqlx::test]
async fn trigger_rejects_project_org_mismatch(pool: PgPool) {
    // 测试 asset_instances.project_id 必须与 organization_id 同组织
}

#[sqlx::test]
async fn valid_project_to_project_same_org_passes(pool: PgPool) {
    // 验证：同项目内的项目级资产依赖是允许的
}

#[sqlx::test]
async fn valid_org_to_org_same_org_passes(pool: PgPool) {
    // 验证：同组织的组织级资产依赖是允许的
}
```

**验收标准**:
- [ ] 迁移测试通过
- [ ] **项目->组织依赖被拒绝测试通过**
- [ ] **组织->项目依赖被拒绝测试通过**
- [ ] **跨项目依赖被拒绝测试通过**
- [ ] **跨组织依赖被拒绝测试通过**
- [ ] **asset_type 组织一致性测试通过**
- [ ] **project 组织一致性测试通过**
- [ ] 部分唯一索引测试通过

#### 任务 3.2: PostgresAssetRepository

**TDD 循环**:

```rust
// RED: 先写集成测试
#[sqlx::test]
async fn postgres_repo_creates_asset(pool: PgPool) {
    let repo = PostgresAssetRepository::new(pool);
    let cmd = CreateAssetCommand {
        name: "Test Asset".to_string(),
        // ...
    };
    
    let asset = repo.create(&cmd).await.unwrap();
    
    // Verify
    let found = repo.find_by_id(&asset.id).await.unwrap().unwrap();
    assert_eq!(found.name, "Test Asset");
}

#[sqlx::test]
async fn postgres_repo_enforces_idempotency(pool: PgPool) {
    let repo = PostgresAssetRepository::new(pool);
    let cmd = CreateAssetCommand {
        // 相同幂等键
        idempotency_key: "git:org1:proj1:abc123".to_string(),
        // ...
    };
    
    repo.create(&cmd).await.unwrap();
    let result = repo.create(&cmd).await; // 第二次应该失败或返回已有资产
    
    assert!(result.is_err() || result.unwrap().idempotency_key == cmd.idempotency_key);
}
```

**验收标准**:
- [ ] CRUD 操作测试通过
- [ ] 幂等性测试通过
- [ ] 组织边界约束测试通过

#### 任务 3.3: PostgresDirtyQueueRepository

**TDD 循环**:

```rust
#[sqlx::test]
async fn upsert_inserts_new_entry(pool: PgPool) {
    let repo = PostgresDirtyQueueRepository::new(pool);
    let entry = DirtyQueueEntry { /* ... */ };
    
    repo.upsert(&entry).await.unwrap();
    
    let found = repo.find_unresolved_by_asset(&entry.asset_id).await.unwrap();
    assert_eq!(found.len(), 1);
}

#[sqlx::test]
async fn resolve_marks_entry_resolved(pool: PgPool) {
    // 先插入，再解决
}

#[sqlx::test]
async fn partial_unique_index_prevents_duplicate_unresolved(pool: PgPool) {
    // 测试 PostgreSQL 部分唯一索引
}
```

**验收标准**:
- [ ] upsert/resolve 测试通过
- [ ] 部分唯一索引行为测试通过

#### 任务 3.4: PostgresDependencyRepository

**TDD 循环**:

```rust
#[sqlx::test]
async fn find_downstream_returns_dependent_assets(pool: PgPool) {
    let repo = PostgresDependencyRepository::new(pool);

    // Setup: B -> A, C -> A
    // source_id 是依赖方/下游，target_id 是被依赖方/上游；
    // 因此 B 和 C 依赖 A 时，A 的 downstream 是 B、C。
    let a_id = AssetId::new();
    let b_id = AssetId::new();
    let c_id = AssetId::new();
    insert_dependency(&pool, b_id, a_id).await; // source=b_id, target=a_id
    insert_dependency(&pool, c_id, a_id).await; // source=c_id, target=a_id

    let downstream = repo.find_downstream(&a_id).await.unwrap();
    assert_eq!(downstream.len(), 2);
    assert!(downstream.contains(&b_id));
    assert!(downstream.contains(&c_id));
}

#[sqlx::test]
async fn find_upstream_returns_dependencies(pool: PgPool) {
    let repo = PostgresDependencyRepository::new(pool);

    // Setup: A -> B, A -> C
    // A 依赖 B 和 C，因此 A 的 upstream 是 B、C。
    let a_id = AssetId::new();
    let b_id = AssetId::new();
    let c_id = AssetId::new();
    insert_dependency(&pool, a_id, b_id).await; // source=a_id, target=b_id
    insert_dependency(&pool, a_id, c_id).await; // source=a_id, target=c_id

    let upstream = repo.find_upstream(&a_id).await.unwrap();
    assert_eq!(upstream.len(), 2);
    assert!(upstream.contains(&b_id));
    assert!(upstream.contains(&c_id));
}

#[sqlx::test]
async fn cycle_detection_trigger_rejects_circular_dependency(pool: PgPool) {
    // Setup: A -> B, B -> C（A 依赖 B，B 依赖 C）
    let a_id = AssetId::new();
    let b_id = AssetId::new();
    let c_id = AssetId::new();

    // 尝试创建 C -> A 形成环
    let result = sqlx::query("INSERT INTO asset_dependencies ...")
        .bind(c_id)
        .bind(a_id)
        .execute(&pool)
        .await;

    assert!(result.is_err());
    // 与 SQL 触发器 BR-006 错误消息一致
    assert!(result.unwrap_err().to_string().contains("BR-006: Cycle detected"));
}
```

**验收标准**:
- [ ] 下游查询测试通过
- [ ] 上游查询测试通过
- [ ] 循环检测触发器测试通过
- [ ] DAG 完整性约束测试通过

---

### Week 4: 应用服务

#### 任务 4.1: AssetService

**TDD 循环**:

```rust
// RED: 写应用服务测试（使用内存仓库）
#[tokio::test]
async fn create_asset_succeeds_with_valid_data() {
    let repo = InMemoryAssetRepository::new();
    let service = AssetService::new(repo);
    
    let cmd = CreateAssetCommand { /* valid data */ };
    let asset = service.create(cmd).await.unwrap();
    
    assert_eq!(asset.current_state, AssetState::Clean);
}

// P2 FIX: 拆分为两个独立测试，明确错误类型
#[tokio::test]
async fn create_asset_fails_for_cross_project_dependency() {
    // Setup: 项目A的资产依赖项目B的资产（同组织，不同项目）
    let org = Organization::new();
    let project_a = Project::new(&org.id);
    let project_b = Project::new(&org.id);
    let asset_in_a = AssetInstance::project_level(&project_a.id);
    let asset_in_b = AssetInstance::project_level(&project_b.id);
    
    let repo = InMemoryAssetRepository::with_data(vec![asset_in_a, asset_in_b]);
    let service = AssetService::new(repo);
    
    let cmd = CreateAssetCommand {
        dependencies: vec![asset_in_b.id], // 跨项目依赖
        // ...
    };
    
    let result = service.create(cmd).await;
    assert!(matches!(result, Err(ServiceError::CrossProjectDependency)));
}

#[tokio::test]
async fn create_asset_fails_for_project_depending_on_organization() {
    // Setup: 项目级资产依赖组织级资产（跨层级）
    let org = Organization::new();
    let project = Project::new(&org.id);
    let project_asset = AssetInstance::project_level(&project.id);
    let org_asset = AssetInstance::organization_level(&org.id);
    
    let repo = InMemoryAssetRepository::with_data(vec![project_asset, org_asset]);
    let service = AssetService::new(repo);
    
    let cmd = CreateAssetCommand {
        dependencies: vec![org_asset.id], // 项目级 -> 组织级（禁止）
        // ...
    };
    
    let result = service.create(cmd).await;
    assert!(matches!(result, Err(ServiceError::ProjectCannotDependOnOrganization)));
}
```

**验收标准**:
- [ ] 创建资产测试通过
- [ ] 边界验证测试通过

#### 任务 4.2: VersionService

**TDD 循环**:

```rust
#[tokio::test]
async fn publish_triggers_dirty_propagation() {
    // Setup: A depends on B
    let asset_repo = InMemoryAssetRepository::with_data(vec![a, b]);
    let dirty_repo = InMemoryDirtyQueueRepository::new();
    let event_bus = InMemoryEventBus::new();
    let service = VersionService::new(asset_repo, dirty_repo, event_bus);
    
    // 发布 B 的新版本
    service.publish(&b.id, "v2.0.0", None).await.unwrap();
    
    // Verify: A 被标记 Dirty
    let a_dirty = dirty_repo.find_unresolved_by_asset(&a.id).await.unwrap();
    assert_eq!(a_dirty.len(), 1);
}

#[tokio::test]
async fn manual_clean_resolves_dirty_state() {
    // ...
}
```

**验收标准**:
- [ ] 发布触发 Dirty 测试通过
- [ ] 版本建议测试通过
- [ ] 手工 Clean 测试通过

---

## Post-MVP: 性能优化（Week 8+）

以下任务不属于 MVP 范围，在核心功能稳定后实施。

### 任务 PM-1: CachedAssetRepository（Redis 缓存层）

**目标**: 为高频查询场景添加缓存支持，提升性能。

**TDD 循环**:

```rust
// RED: 先写缓存失败场景测试
#[tokio::test]
async fn find_by_id_succeeds_when_redis_fails() {
    // Setup: Redis 失败，但数据库可用
    let inner = InMemoryAssetRepository::with_data(vec![asset]);
    let redis = FailingRedisConnection::new("Connection refused");
    let cached = CachedAssetRepository::new(inner, redis, Duration::from_secs(60));

    // 应该成功从底层仓库获取
    let result = cached.find_by_id(&asset.id).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap().unwrap().name, asset.name);
}

#[tokio::test]
async fn save_succeeds_when_redis_cache_update_fails() {
    // Setup: Redis 写入失败
    let inner = InMemoryAssetRepository::new();
    let redis = FailingRedisConnection::new("Write timeout");
    let cached = CachedAssetRepository::new(inner, redis, Duration::from_secs(60));

    let cmd = CreateAssetCommand { /* ... */ };

    // 数据库写入应成功，Redis 失败只记录 warning
    let result = cached.create(&cmd).await;
    assert!(result.is_ok());

    // Verify: 资产已创建（即使 Redis 失败）
    let asset = cached.find_by_id(&result.unwrap().id).await.unwrap().unwrap();
    assert_eq!(asset.name, cmd.name);
}

#[tokio::test]
async fn cache_hit_returns_cached_value() {
    // Setup: Redis 可用
    let inner = InMemoryAssetRepository::with_data(vec![asset]);
    let redis = InMemoryRedis::new();
    let cached = CachedAssetRepository::new(inner, redis, Duration::from_secs(60));

    // 第一次查询，写入缓存
    let _ = cached.find_by_id(&asset.id).await;

    // 第二次查询，应从缓存返回（不查数据库）
    let result = cached.find_by_id(&asset.id).await;
    assert!(result.is_ok());
}
```

**验收标准**:
- [ ] Redis 失败时 find_by_id 仍成功测试通过
- [ ] Redis 失败时 save 仍成功测试通过
- [ ] 缓存命中测试通过
- [ ] 缓存失效测试通过
- [ ] 缓存降级不阻塞主流程

**依赖**:
- `redis = { version = "0.29", features = ["tokio-comp"] }`
- Infrastructure crate 添加 `cache` feature

---

## 新增：Phase 4.5 - 安全与授权（Week 4.5）

**P1 补充**: 项目成员关系授权

### Week 4.5: 项目成员关系与权限

#### 任务 4.5.1: AuthPrincipal（与 architecture.md 一致）

**P1 FIX**: 与 architecture.md 一致，`project_memberships` 内嵌在 `AuthPrincipal` 中

**TDD 循环**:

```rust
// RED: 先写 AuthPrincipal 测试（与 architecture.md 定义一致）
#[test]
fn auth_principal_can_check_project_membership() {
    let principal = AuthPrincipal {
        id: "user-123".to_string(),
        principal_type: PrincipalType::User,
        organization_id: org_id!("org-1"),
        roles: vec![Role::Developer],
        project_memberships: vec![project_id!("project-1"), project_id!("project-2")],
        metadata: AuthMetadata::default(),
    };
    
    // 检查是否是项目成员
    assert!(principal.is_member_of(&project_id!("project-1")));
    assert!(!principal.is_member_of(&project_id!("project-3")));
}

#[test]
fn developer_has_project_permissions_in_member_projects() {
    let principal = AuthPrincipal {
        id: "user-123".to_string(),
        roles: vec![Role::Developer],
        project_memberships: vec![project_id!("project-1")],
        // ...
    };
    
    // Developer 在自己所属项目有 AssetCreate 权限
    assert!(principal.has_permission_for_project(
        Permission::AssetCreate,
        &project_id!("project-1")
    ));
    
    // Developer 在非所属项目没有权限（除非 OrgAdmin）
    assert!(!principal.has_permission_for_project(
        Permission::AssetCreate,
        &project_id!("project-2")
    ));
}

#[test]
fn org_admin_can_access_any_project_in_org() {
    let principal = AuthPrincipal {
        roles: vec![Role::OrgAdmin],
        project_memberships: vec![], // 不一定是任何项目成员
        // ...
    };
    
    // OrgAdmin 可以访问组织内任何项目
    assert!(principal.has_permission_for_project(
        Permission::AssetCreate,
        &project_id!("project-1") // 非成员项目
    ));
}

#[test]
fn system_admin_can_access_same_org_without_project_membership() {
    let principal = AuthPrincipal {
        roles: vec![Role::SystemAdmin],
        organization_id: org_id!("org-1"),
        project_memberships: vec![],
    };
    
    // SystemAdmin 在同组织内可绕过项目成员列表；跨组织仍由 AuthorizationService 拒绝
    assert!(principal.can(Permission::AssetCreate, org_id!("org-1"), None));
}
```

**实现步骤**:
1. [ ] RED: 写 AuthPrincipal 测试
2. [ ] GREEN: 实现 AuthPrincipal、is_member_of、has_permission_for_project
3. [ ] REFACTOR: 提取权限检查逻辑

**验收标准**:
- [ ] AuthPrincipal 结构体与 architecture.md 一致
- [ ] is_member_of 测试通过
- [ ] Developer 项目权限测试通过
- [ ] OrgAdmin 豁免测试通过
- [ ] SystemAdmin 同组织内项目成员资格豁免测试通过

#### 任务 4.5.2: 授权服务（使用内嵌 memberships）

**TDD 循环**:

```rust
// RED: 先写授权服务测试（使用 principal.project_memberships）
#[tokio::test]
async fn non_project_member_cannot_access_project_assets() {
    let auth_service = AuthorizationService::new();
    let principal = AuthPrincipal {
        id: "user-123".to_string(),
        principal_type: PrincipalType::User,
        organization_id: org_id!("org-1"),
        roles: vec![Role::Developer], // 不是 ProjectAdmin
        project_memberships: vec![project_id!("project-1")], // 只属于 project-1
        metadata: AuthMetadata::default(),
    };
    
    // 尝试访问 project-2 的资产
    let result = auth_service.check(
        &principal,
        Permission::AssetRead,
        org_id!("org-1"),
        Some(project_id!("project-2")), // 非成员项目
    ).await;
    
    // Developer 访问非成员项目被拒绝
    assert!(matches!(result, Err(AuthorizationError::ProjectAccessDenied(_))));
}

#[tokio::test]
async fn project_member_can_access_project_assets() {
    let principal = AuthPrincipal {
        id: "user-123".to_string(),
        principal_type: PrincipalType::User,
        organization_id: org_id!("org-1"),
        roles: vec![Role::Developer],
        project_memberships: vec![project_id!("project-1")],
        metadata: AuthMetadata::default(),
    };
    
    // 访问成员项目
    let result = auth_service.check(
        &principal,
        Permission::AssetRead,
        org_id!("org-1"),
        Some(project_id!("project-1")),
    ).await;
    
    assert!(result.is_ok());
}

#[tokio::test]
async fn org_admin_can_access_any_project_in_org_without_membership() {
    let principal = AuthPrincipal {
        id: "user-123".to_string(),
        principal_type: PrincipalType::User,
        organization_id: org_id!("org-1"),
        roles: vec![Role::OrgAdmin],
        project_memberships: vec![], // 不是任何项目成员
        metadata: AuthMetadata::default(),
    };
    
    // OrgAdmin 可以访问任何项目
    let result = auth_service.check(
        &principal,
        Permission::AssetRead,
        org_id!("org-1"),
        Some(project_id!("project-1")),
    ).await;
    
    assert!(result.is_ok());
}

#[tokio::test]
async fn system_admin_can_access_same_org_without_project_membership() {
    let principal = AuthPrincipal {
        id: "admin".to_string(),
        principal_type: PrincipalType::User,
        organization_id: org_id!("org-1"),
        roles: vec![Role::SystemAdmin],
        project_memberships: vec![],
        metadata: AuthMetadata::default(),
    };
    
    // SystemAdmin 仍受组织边界约束，但不需要逐项列出项目成员关系
    let result = auth_service.check(
        &principal,
        Permission::AssetDelete, // 任何权限
        org_id!("org-1"),         // 同组织
        Some(project_id!("any-project")),
    ).await;
    
    assert!(result.is_ok());
}

#[tokio::test]
async fn system_admin_cannot_cross_organization_boundary() {
    let principal = AuthPrincipal {
        id: "admin".to_string(),
        principal_type: PrincipalType::User,
        organization_id: org_id!("org-1"),
        roles: vec![Role::SystemAdmin],
        project_memberships: vec![],
        metadata: AuthMetadata::default(),
    };

    let result = auth_service.check(
        &principal,
        Permission::AssetRead,
        org_id!("org-2"),
        None,
    ).await;

    assert!(matches!(result, Err(AuthorizationError::CrossOrganizationAccessDenied)));
}
```

**验收标准**:
- [ ] 非项目成员访问返回 403 测试通过
- [ ] 项目成员访问成功测试通过
- [ ] OrgAdmin 无需成员资格即可访问测试通过
- [ ] SystemAdmin 在同组织内无需成员资格即可访问测试通过
- [ ] SystemAdmin 跨组织访问被拒绝测试通过
- [ ] **不使用 membership_repo 查询**，使用 principal.project_memberships

#### 任务 4.5.3: REST API 权限集成

**TDD 循环**:

```rust
#[tokio::test]
async fn api_returns_403_for_non_member() {
    let app = create_test_app().await;
    
    // Token 中包含 principal.project_memberships
    let token = generate_token_for_user(AuthPrincipal {
        id: "user-123".to_string(),
        roles: vec![Role::Developer],
        project_memberships: vec![project_id!("project-1")], // 只属于 project-1
        // ...
    });
    
    let response = app
        .oneshot(Request::builder()
            .method(Method::GET)
            .uri("/api/v1/assets?project_id=project-2") // 访问 project-2
            .header("authorization", format!("Bearer {}", token))
            .body(Body::empty())
            .unwrap())
        .await
        .unwrap();
    
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}
```

**验收标准**:
- [ ] REST API 权限检查使用 principal.project_memberships 测试通过
- [ ] MCP 权限检查使用 principal.project_memberships 测试通过
- [ ] 错误信息包含权限详情

---

## 第三阶段：REST API（Week 5）

#### 任务 5.1: Handler 测试（端到端）

**TDD 循环**:

```rust
// RED: 先写 handler 测试
#[tokio::test]
async fn create_asset_endpoint_returns_201() {
    let app = create_test_app().await;
    
    let response = app
        .oneshot(Request::builder()
            .method(Method::POST)
            .uri("/api/v1/assets")
            .header("authorization", "Bearer test-token")
            .body(Body::from(json!({
                "name": "Test Asset",
                "type_id": "...",
                // ...
            }).to_string()))
            .unwrap())
        .await
        .unwrap();
    
    assert_eq!(response.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn create_asset_without_auth_returns_401() {
    // ...
}
```

**验收标准**:
- [ ] 主要端点测试通过
- [ ] 认证测试通过
- [ ] 错误处理测试通过

---

## 第四阶段：MCP Server（Week 6-7）

#### 任务 6.1: MCP Tools

**TDD 循环**:

```rust
// RED: 先写 MCP Tool 测试
#[tokio::test]
async fn query_assets_tool_returns_assets() {
    let server = create_test_mcp_server().await;
    
    let result = server
        .query_assets("project-123".to_string(), None, None, None)
        .await
        .unwrap();
    
    // Verify result contains assets
}

#[tokio::test]
async fn create_virtual_asset_with_invalid_anchor_returns_error() {
    let server = create_test_mcp_server().await;
    
    let result = server
        .create_virtual_asset(
            "requirement".to_string(),
            vec!["invalid-uuid".to_string()], // 无效 ID
            "project-123".to_string(),
        )
        .await;
    
    assert!(result.is_err()); // 应该返回 McpError，不 panic
}
```

**验收标准**:
- [ ] query_assets Tool 测试通过
- [ ] create_virtual_asset 测试通过（含错误处理）
- [ ] 权限检查测试通过

---

## 测试覆盖率检查点

### Week 1 结束
```bash
$ cargo tarpaulin --packages adam-domain --out Stdout
# 目标: > 80%
# 检查: AssetState, DAGValidator, DependencyBoundaryContext
```

### Week 2 结束
```bash
$ cargo tarpaulin --packages adam-domain --out Stdout
# 目标: > 85%
# 检查: StatePropagator, DirtyQueueEntry, 所有 Repository trait
```

### Week 4 结束
```bash
$ cargo tarpaulin --workspace --out Stdout
# 目标: > 80%
# 检查: AssetService, VersionService, Postgres 实现
```

### Week 7 结束
```bash
$ cargo tarpaulin --workspace --out Stdout
# 目标: > 80%
# 检查: REST handlers, MCP tools, 集成测试
```

---

## 每日 TDD 流程

### 开发人员每日工作流

```
1. 选择任务
   ↓
2. RED: 写测试（预期失败）
   cargo test --lib <test_name>  # 应该失败
   ↓
3. GREEN: 写最少实现
   修改代码直到测试通过
   cargo test --lib <test_name>  # 应该通过
   ↓
4. 写下一个测试或 REFACTOR
   ↓
5. 运行全部测试
   cargo test --package <crate>
   ↓
6. 提交（Commit）
   git add .
   git commit -m "feat(domain): AssetState 状态转换
   
   - 添加 can_transition_to 方法
   - 覆盖 Clean->Dirty, Dirty->Clean, Archived 边界
   
   Tests: cargo test asset_state
   Coverage: 100%"
```

### Commit 信息规范

```
<scope>: <description>

<body>

Tests: <how to run tests>
Coverage: <percentage>
```

示例:
```
feat(domain): 实现 DependencyBoundaryContext

实现依赖边界验证，支持：
- 同项目内项目级资产依赖
- 同组织内组织级资产互依赖
- 禁止跨项目依赖（BR-008）
- 禁止项目级与组织级互依赖（BR-008）

Refs: architecture.md#363

Tests: cargo test dependency_boundary
Coverage: 95%
```

---

## CI/CD 中的 TDD

### CI 流程

```yaml
# .github/workflows/ci.yml
jobs:
  test:
    steps:
      - name: Checkout
        uses: actions/checkout@v4
      
      - name: Setup PostgreSQL
        uses: ikalnytskyi/action-setup-postgres@v4
      
      - name: Run migrations
        run: sqlx migrate run
      
      - name: Test (TDD 验证)
        run: |
          # 强制要求测试通过
          cargo test --workspace --all-features
          
          # 覆盖率检查
          cargo tarpaulin --fail-under 80
      
      - name: Clippy
        run: cargo clippy -- -D warnings
      
      - name: Format check
        run: cargo fmt --check
```

---

## 附录：TDD 快速参考

### 测试命名规范

| 前缀 | 含义 | 示例 |
|------|------|------|
| `test_<fn>_<scenario>` | 函数测试 | `test_can_transition_rejects_invalid` |
| `test_<module>_<behavior>` | 模块测试 | `test_propagation_creates_dirty_entries` |
| `integration_<feature>` | 集成测试 | `integration_asset_lifecycle` |
| `e2e_<workflow>` | E2E 测试 | `e2e_publish_and_propagate_dirty` |

### 断言模式

```rust
// 成功断言
assert!(result.is_ok());
assert_eq!(actual, expected);

// 错误断言
assert!(matches!(result, Err(ExpectedError)));
assert!(result.is_err());

// 集合断言
assert_eq!(vec.len(), 2);
assert!(vec.contains(&item));
```

### Mock 模式

```rust
// 内存仓库实现用于测试
struct InMemoryAssetRepository {
    data: Mutex<HashMap<AssetId, AssetInstance>>,
}

impl AssetRepository for InMemoryAssetRepository {
    async fn create(&self, cmd: &CreateAssetCommand) -> Result<AssetInstance, RepositoryError> {
        // 简单内存存储
    }
}
```

---

*文档结束 - 遵循 TDD：先写测试，后写实现，持续重构*
