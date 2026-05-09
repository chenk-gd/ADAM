# ADAM 系统实现审计报告

**日期**: 2026-05-09
**版本**: spec v1.8 / architecture v1.1 / implementation-plan v1.2

---

## 1. 总体评估

| 维度 | 完成度 | 评级 |
|------|--------|------|
| 规格覆盖 (26 个 FR) | ~35% | C |
| 架构合规性 | ~60% | B- |
| 代码质量 | ~70% | B |
| 测试覆盖 | ~40% | C+ |
| 数据库/迁移 | ~80% | A- |

**总评**: 核心骨架已搭建（6,650 行 Rust），领域模型基本正确，但大量功能需求仍为桩实现或完全缺失。MVP 可用的关键路径（资产CRUD + 发布传播 + Dirty解析）有端到端链路，但版本管理、资产类型、依赖规则等核心模块缺失严重。REST API 仅实现规格约 30 个端点中的 5 个（~17% 实现率）。

---

## 2. 规格需求 vs 实现对照

### 2.1 已实现的需求

| 需求 | 状态 | 实现位置 | 备注 |
|------|------|----------|------|
| FR-005 资产实例创建 | ✅ 部分 | `rest/mod.rs` create_asset, `repository/in_memory.rs`, `postgres.rs` | 缺少 metadata/source/assignees 字段 |
| FR-006 资产实例查询 | ✅ 部分 | `rest/mod.rs` list_assets, get_asset | 缺少分页、排序、按类型/状态/时间过滤 |
| FR-007 资产实例更新 | ❌ 缺失 | 无 PUT handler | 规格要求支持非版本化字段更新 |
| FR-008 资产实例删除 | ❌ 缺失 | 无 DELETE handler | 规格要求检查下游依赖后软删除 |
| FR-015 资产状态定义 | ✅ 完成 | `asset/state.rs` AssetState enum | Clean/Dirty/Archived 三态正确 |
| FR-016 状态自动转换 | ✅ 部分 | `services/state_propagator.rs` | 发布触发Dirty传播已实现；手工Clean仅在REST层部分实现 |
| FR-017 状态传播规则 | ✅ 部分 | `services/state_propagator.rs` | 仅实现发布→直接下游Dirty；缺手工Clean→不触发下游、并发合并、优先级计算 |
| FR-022 虚拟实例 | ✅ 部分 | `virtual_instance.rs`, `mcp/mod.rs` create_virtual_asset/get_virtual_context | 创建和查询已实现，缺按预设依赖规则自动推导 |
| FR-023 MCP Server | ✅ 部分 | `mcp/mod.rs` AdanMcpServer | 10个Tool全部注册，但多个为桩实现 |
| FR-025 权限控制 | ✅ 部分 | `auth.rs`, `rest/mod.rs` 授权检查 | RBAC 基本完成，缺速率限制和审计日志 |
| FR-026 项目/组织层级 | ✅ 完成 | `dependency/boundary.rs`, REST list_assets | 查询合并项目+组织资产已实现 |

### 2.2 未实现的需求

> 注：规格文档 `spec.md` 实际列出 FR-001 ~ FR-027 共 **26 个** 功能需求（FR-014 在文档中缺失）。以下按实际出现的 FR 编号列出。

| 需求 | 优先级 | 影响 | 说明 |
|------|--------|------|------|
| FR-001 资产类型定义 | 高 | 严重 | 无 AssetType 领域实体、CRUD API、元数据 schema 验证。`AssetTypeId` 已存在，但 `AssetType` 实体缺失 |
| FR-002 资产类型元数据 | 中 | 高 | 无 JSON Schema 定义和校验 |
| FR-003 依赖关系配置 | 高 | 严重 | 无 DependencyRule 领域实体和管理接口，预设依赖规则缺失 |
| FR-004 依赖关系验证 | 中 | 高 | 创建实例时不验证依赖是否符合预设规则 |
| FR-009 实例依赖建立 | 高 | 严重 | 发布时不建立/快照依赖关系；REST create_asset 有 dependencies 字段但标注 TODO |
| FR-010 依赖关系查询 | 高 | 高 | 仅 AssetRepository 有 find_upstream/downstream，REST 无依赖图端点 |
| FR-011 依赖关系变更 | 中 | 中 | 无依赖变更管理（手工 Clean 基线更新等） |
| FR-012 版本发布管理 | 高 | 严重 | 无 AssetVersion 实体、无版本快照、SemVer 建议为硬编码桩 |
| FR-013 版本历史与对比 | 中 | 高 | 完全缺失 |
| FR-018 外部内容存储 | 高 | 中 | MCP get_asset_content 为占位实现（返回提示文本） |
| FR-019 外部系统接入 | 高 | 严重 | 无 Webhook/Git Hooks/CI 集成 |
| FR-019A Git 自动化 | 高 | 严重 | 完全缺失 |
| FR-019B CI/CD 集成 | 中 | 高 | PipelineRun 实体和保留策略缺失 |
| FR-020 内容获取接口 | 高 | 中 | 仅有外部引用概念，无实际代理/重定向 |
| FR-021 多维度查询 | 高 | 高 | 缺少按责任人/时间/关键词/深度搜索 |
| FR-024 Skill 场景封装 | 中 | 中 | 无 Skill 层实现 |
| FR-027 版本保留策略 | 中 | 中 | 完全缺失 |

---

## 3. 架构合规性

### 3.1 合规项 ✅

- **六边形架构分层**: domain → application → infrastructure → adapters → server 依赖方向正确
- **领域层独立性**: adam-domain 仅依赖 serde/chrono/uuid/thiserror/async-trait，无框架依赖
- **Repository 接口定义**: AssetRepository/DirtyQueueRepository/DependencyRepository/VirtualInstanceRepository 定义在 domain 层
- **依赖方向不变量**: `source_id`=下游, `target_id`=上游 在 postgres.rs SQL 查询中正确实现
- **状态存储不变量**: `current_state` 仅存枚举标签，Dirty详情在 `dirty_queue` 表
- **层级边界不变量**: `DependencyBoundaryContext::validate()` 正确实现 BR-008 规则
- **DAG 验证**: `DAGValidator` 使用 petgraph 实现，支持泛型
- **发布事务概念**: `StatePropagator::on_asset_published` 实现了发布→Dirty传播链路

### 3.2 不合规项 ❌

| 问题 | 严重度 | 说明 |
|------|--------|------|
| **REST 层重复定义 AuthPrincipal** | 高 | `rest/mod.rs` 定义了自己的 AuthPrincipal，而非复用 domain 的 `auth.rs` 版本。两套结构会漂移 |
| **MCP 层缺少 AuthorizationError 映射** | 低 | MCP 定义了自己的 `AuthorizationError`，但实际从 domain 映射，问题不大 |
| **Application 层仅有一个服务** | 中 | `adam-application` 只有 StatePropagator，缺少 AssetService/VersionService/ImpactService 等 |
| **Infrastructure 层 InMemoryDependencyRepository 重复** | 低 | `adam-infrastructure/src/repositories/mod.rs` 和 `adam-adapters/src/mcp/mod.rs` 各有一个测试用 InMemoryDependencyRepository |
| **CORS 完全开放** | 安全 | `CorsLayer::new().allow_origin(Any)` 不适合生产 |
| **MCP publish_asset 为桩实现** | 严重 | MCP 的 publish_asset 未调用 StatePropagator，返回硬编码响应 |
| **MCP manual_clean_asset 为桩实现** | 严重 | 未实际更新状态或创建 DirtyResolutionLog |
| **MCP refresh_asset_state 为桩实现** | 高 | `upstream_changes` 硬编码为 true，previous_state=current_state |
| **MCP suggest_version 为桩实现** | 中 | 版本建议硬编码，不读取当前版本 |
| **clippy 警告未修复** | 低 | postgres.rs 有2个 uninlined_format_args 警告 |

---

## 4. 代码质量

### 4.1 优点

- **Newtype 模式**: AssetId/AssetTypeId/ProjectId/OrganizationId 使用 newtype，防止类型混淆
- **不可变默认**: Rust 变量默认不可变，`let mut` 仅在需要时使用
- **错误处理**: 使用 thiserror 定义领域错误，? 传播，无明显 unwrap 滥用
- **模块组织**: 按领域（asset/dependency/repository）而非按类型组织
- **单元测试内联**: 各模块有 `#[cfg(test)]` 内联单元测试

### 4.2 问题

| 问题 | 位置 | 建议 |
|------|------|------|
| AssetType 实体缺失 | `asset/mod.rs` | 仅有 `AssetTypeId` (newtype)，无 `AssetType` 领域实体及元数据 schema 字段 |
| AssetInstance 缺少关键字段 | `instance.rs` | 缺 external_ref, source, metadata, assignees, publisher, current_version |
| DirtyQueueEntry 的 created_at 类型不一致 | `repository/mod.rs` | 域模型用 `DateTime<Utc>`，DB用 `NaiveDateTime`（postgres.rs 隐式转换） |
| REST AuthPrincipal 与 Domain AuthPrincipal 重复 | `rest/mod.rs:31-37` | 应删除 REST 版本，使用 domain 的 AuthPrincipal |
| MCP call_tool 重复参数解析模式 | `mcp/mod.rs:1074-1275` | 10个工具的参数解析代码几乎相同，应提取宏或辅助函数 |
| InMemoryAssetRepository.update_state 直接修改 | `in_memory.rs:90` | 违反不可变原则：`data.get_mut(id)` 应返回新实例 |
| CORS Any 配置 | `rest/mod.rs:632` | 生产环境需限制 origin |
| list_tools 手动构建 schema | `mcp/mod.rs:1287-1335` | 10次重复 schemars::schema_for! + serde_json 转换 |

---

## 5. 测试覆盖

### 5.1 测试统计

| 模块 | 单元测试数 | 集成测试数 | 覆盖评估 |
|------|-----------|-----------|---------|
| adam-domain/asset/state | 4 | 0 | 良好 |
| adam-domain/asset/instance | 3 | 0 | 基础 |
| adam-domain/dependency/dag | 7 | 1 (integration) | 良好 |
| adam-domain/dependency/boundary | 6 | 1 (integration) | 良好 |
| adam-domain/virtual_instance | 2 | 0 | 基础 |
| adam-domain/auth | 8 | 0 | 良好 |
| adam-domain/repository (in_memory) | 0 | 1 (integration) | 不足 |
| adam-application/state_propagator | 5 | 0 | 良好 |
| adam-infrastructure/repositories | 3 | 0 | 不足（Postgres测试全ignore） |
| adam-infrastructure/postgres | 0 | 1 (ignored) | 严重不足 |
| adam-adapters/rest | 14 | 0 | 良好（但仅测试happy path和auth） |
| adam-adapters/mcp | 9 | 0 | 中等（桩实现未覆盖实际逻辑） |

**估算覆盖率**: ~40-50%（目标 80%）

> 精确统计：代码库共 **6,650 行 Rust 代码**，包含 **101 个测试函数**（`#[test]` + `#[tokio::test]`），其中 Postgres 集成测试全部 `#[ignore]`。

### 5.2 缺失的关键测试

- [ ] AssetInstance 缺少状态转换的边界测试（Archived 不应发布）
- [ ] StatePropagator 缺少并发发布场景测试
- [ ] DirtyQueueRepository 缺少 upsert 合并逻辑测试
- [ ] VirtualInstanceRepository 缺少过期清理测试
- [ ] Postgres 集成测试全部 ignored，无实际 DB 测试
- [ ] REST 缺少跨组织访问、分页、排序测试
- [ ] MCP 缺少 publish_asset/manual_clean/refresh_state 的实际逻辑测试（当前为桩）
- [ ] 无端到端测试

---

## 6. 数据库/迁移评估

### 6.1 已完成的迁移

| 迁移 | 表/功能 | 评估 |
|------|---------|------|
| 001 | organizations, projects | ✅ 完整 |
| 002 | asset_types, dependency_rules | ✅ 完整 |
| 003 | asset_instances | ✅ 完整 |
| 004 | asset_dependencies | ✅ 完整（含 declared/effective version） |
| 005 | dirty_queue | ✅ 完整（含部分唯一索引） |
| 006 | asset_versions, dirty_resolution_logs | ✅ 完整 |
| 007 | triggers_and_constraints | ✅ 完整（DAG/层级/组织边界校验） |

### 6.2 问题

- Postgres 仓库实现缺少 `AssetVersionRepository`、`DirtyResolutionLogRepository`
- `find_by_organization_id` 查询返回所有层级资产，REST 层需二次过滤（应在 SQL 层 `WHERE level = 'organization'`）

---

## 7. 优先修复建议

### P0 - 阻塞 MVP

1. **实现 AssetType CRUD**（FR-001/FR-002）- 没有资产类型，系统无法运行
2. **实现 DependencyRule 管理**（FR-003）- 没有预设依赖规则，无法验证实例依赖
3. **实现 AssetVersion 实体和发布流程**（FR-012）- 当前 publish 仅为状态传播，无版本记录
4. **修复 MCP publish_asset 调用 StatePropagator** - REST 已正确调用，MCP 未调用
5. **修复 REST/AuthPrincipal 重复定义** - 统一使用 domain 层定义

### P1 - 关键功能

6. **实现实例依赖建立**（FR-009）- 发布时建立并快照依赖关系
7. **实现 REST PUT/DELETE assets**（FR-007/FR-008）
8. **实现 REST 依赖关系端点**（FR-010/FR-011）
9. **实现手工 Clean 完整流程** - 含 DirtyResolutionLog 创建
10. **添加分页/排序/过滤**（FR-006/FR-021）

### P2 - 质量和安全

11. **修复 clippy 警告**
12. **提取 MCP 参数解析为宏/辅助函数**
13. **限制 CORS origin**
14. **补充 Postgres 集成测试**
15. **添加速率限制和审计日志**

---

## 附录：核查验证

本报告已通过自动化脚本逐条核对代码实际状态，验证结果如下：

| 报告声明 | 验证方法 | 结果 |
|---------|---------|------|
| REST AuthPrincipal 与 Domain 重复定义 | 检查 `rest/mod.rs` 和 `domain/auth.rs` | ✅ 确认重复 |
| AssetInstance 缺 6 个字段 | 逐一检查字段名在 `instance.rs` 中存在性 | ✅ 全部缺失 |
| MCP publish_asset/manual_clean/refresh 为桩 | 检查函数体内是否含 StatePropagator 调用或 TODO | ✅ 确认桩实现 |
| CORS allow_origin(Any) | 检查 `rest/mod.rs` CorsLayer 配置 | ✅ 确认不安全 |
| DependencyRule 不存在 | 检查 domain 层符号 | ✅ 确认缺失 |
| AssetVersion 无文件 | 遍历 domain src 目录 | ✅ 确认缺失 |
| REST PUT/DELETE 缺失 | 检查 router 定义 | ✅ 确认缺失 |
| Postgres 测试全部 ignore | 检查 `postgres.rs` 测试属性 | ✅ 确认 |
| AssetTypeId 存在但 AssetType 实体缺失 | 检查 `asset/mod.rs` | ✅ 确认（Id 存在，实体缺失） |
| list_tools 10 次重复 schema_for! | 统计 `mcp/mod.rs` 调用次数 | ✅ 确认 10 次 |
| 规格 FR 总数 26 个 | 统计 `spec.md` 中 `#### FR-` 出现次数 | ✅ 确认（FR-001~FR-027，FR-014 缺失） |
| 总 Rust 代码量 | 统计所有 `.rs` 文件行数 | 6,650 行 |
| 总测试函数数 | 统计所有 `#[test]` 和 `#[tokio::test]` | 101 个 |
| REST API 端点实现率 | 对比 router 与 spec 6.x 节端点列表 | 5/30 ≈ 17% |
