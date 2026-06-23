# Asset-Driven Workflow Automation — Implementation Plan

> 依据设计文档 [`2026-06-15-asset-driven-workflow-automation-design.md`](./2026-06-15-asset-driven-workflow-automation-design.md) 制定。
> 目标：按 Slice 1 → 2 → 3 增量交付，每个 Slice 独立可测、可发布，遵循项目既有 DDD 分层与 TDD 约定。

## 0. 交付约束与通用约定

### 0.1 分层落点（与现有 crate 对齐）

| 关注点 | Crate | 路径约定 |
| --- | --- | --- |
| 实体 / 值对象 / 状态机 / 仓库 trait | `adam-domain` | `src/workflow/`（新模块）、`src/repository/mod.rs` 追加 trait |
| 应用服务（用例编排） | `adam-application` | `src/services/workflow/`（新子模块） |
| Postgres 仓库实现 | `adam-infrastructure` | `src/repositories/postgres.rs` 追加实现 |
| REST / MCP 适配 | `adam-adapters` | `src/rest/workflow.rs`、`src/mcp/workflow.rs` |
| 装配 / 运行 | `adam-server` | `src/main.rs` 注入依赖 |
| 数据库迁移 | `migrations/` | `014_workflow_automation.sql`（仅新增，不改既有迁移） |

### 0.2 必须遵守的既有模式

- **仓库 trait**：使用 `#[async_trait::async_trait]`；写操作采用 CAS（`update_*_cas(id, expected_lock_version, ...)`），冲突返回 `RepositoryError::ConcurrentModification`。
- **幂等**：复用 `adam-domain::idempotency`（`IdempotencyKey` / `IdempotencyRecord` / `IdempotencyRepository`）。事件与动作的幂等键来自设计文档第 9 节：
  - 规则→动作：`rule_id + event_id + target_asset_id`
  - 动作实例：`workflow_instance_id + action_type + target_asset_id`
- **错误**：领域错误用 `thiserror`，仓库错误统一 `RepositoryError`；适配层映射为 HTTP/MCP 错误。
- **不可变值对象 + 显式状态机**：状态转换在 service 层显式校验，非法转换返回领域错误且不发事件。
- **TDD**：每个仓库 trait 先在 `in_memory` 实现并配单测，再写 postgres 实现 + 集成测试；service 层用 `mockall` mock 仓库。

### 0.3 测试策略

| 层级 | 类型 | 位置 | 工具 |
| --- | --- | --- | --- |
| 领域 | 单元（状态机、规则评估、幂等键） | `crates/adam-domain/tests/workflow_*.rs` | 内置 `#[test]` |
| 应用 | 单元（service 用 mockall） | `crates/adam-application/src/services/workflow/` 同级 `#[cfg(test)]` | `mockall` |
| 基础设施 | 集成（真实 Postgres，TestContainer 或本地 PG） | `crates/adam-infrastructure/tests/` | `sqlx`、`tokio::test` |
| 适配 | 端到端（REST/MCP） | `tests/` | `axum::test`、`rmcp` client |

> 覆盖率目标 80%+；每个 Slice 的验收标准必须全部有对应测试。

### 0.4 TDD 工作流（强制，每个任务）

每个任务（`Sn-Tx`）严格按 RED → GREEN → REFACTOR → GATE 四阶段执行，**不得跳过**：

1. **RED — 先写测试**
   - 在写任何实现代码前，先写失败测试：领域逻辑写 `tests/workflow_*.rs`，service 写同文件 `#[cfg(test)] mod tests`，仓库写 `in_memory` 单测 + infra 集成测试骨架。
   - 测试必须可编译通过编译期检查，但断言必然失败。
   - 运行 `cargo test --workspace <相关测试>` 确认 **RED**（看到断言失败，而非编译失败）。
   - 一条原则：测试名表达行为意图，如 `replay_same_event_does_not_create_duplicate_action`。

2. **GREEN — 最小实现使测试通过**
   - 写**最小**的实现让所有相关测试变绿，不提前抽象、不加未测功能。
   - 再跑 `cargo test --workspace <相关测试>` 确认全绿。

3. **REFACTOR — 在测试保护下重构**
   - 提取重复、对齐既有风格、消除 clippy 警告。
   - 每次重构后立即 `cargo test`，保持全绿；一旦变红立即回退。

4. **GATE — 任务完成规格与质量检查**（见 §0.5）

> TDD 纪律：若一个任务无法写出失败测试（如纯迁移文件），则改为「迁移 + 校验脚本/集成断言」作为该任务的测试锚点，并在任务说明中注明。

### 0.5 任务完成质量门（每个 `Sn-Tx` 完成时强制执行）

每个任务在标记完成前，**逐项**核对以下清单，全部满足方可进入下一任务。任一未过则回 RED 修正：

**规格符合性（Spec）**
- [ ] 实现对照设计文档对应章节（任务头部标注的 §x），行为与设计描述一致。
- [ ] 非法状态转换返回领域错误且不发事件（设计 §5）。
- [ ] 幂等键与事务边界符合设计 §9/§10。
- [ ] 命名、错误类型、值对象与既有 crate 风格一致（不可变模式、`thiserror`、`#[async_trait]`）。
- [ ] 没有引入设计 §18 Non-Goals 范畴（BPMN 引擎、可视化编辑、跨项目传播、自动合并部署、无限制 Agent 自治、新增 `defect_report`/`test_report` 类型）。

**质量检查（Quality）**
- [ ] `cargo fmt --all` 无改动。
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` 零警告。
- [ ] `cargo test --workspace` 全绿；本任务新增/相关测试 100% 通过。
- [ ] 单文件 ≤ 400 行（领域/服务），复杂适配器 ≤ 800 行；函数 ≤ 50 行；嵌套 ≤ 4 层。
- [ ] 无 `unwrap()`/`expect()` 进入生产路径（测试代码除外）；错误显式处理。
- [ ] 无硬编码值（连接串、阈值、间隔等走配置或常量）。
- [ ] 无秘密/凭证硬编码；外部输入在边界校验。
- [ ] 覆盖率：本任务涉及代码 ≥ 80%（可用 `cargo tarpaulin` 或 `cargo llvm-cov` 抽查关键模块）。

**影响与文档（Impact & Docs）**
- [ ] `gitnexus_impact({target: "<改动符号>", direction: "upstream"})` 评估 blast radius；HIGH/CRITICAL 须告知并更新 d=1 依赖方。
- [ ] 若任务涉及 REST/MCP 端点：`docs/rest-api.md` 与 `docs/openapi.yaml` 同步更新。
- [ ] 若任务改变领域语义：在本计划任务说明或 ADR 记录决策。
- [ ] 提交前 `gitnexus_detect_changes()` 确认改动范围与预期一致。
- [ ] 任务说明末尾追加一行：`> 质量门：✅ 通过（附命令输出摘要）` 或 `❌ 未过（原因）`。

> 质量门不通过不得合入分支。Slice 合并前还需通过 §0.6 的整 Slice 门禁。

### 0.6 Slice 合并门禁

每个 Slice 合并到 `main` 前，在 §0.5 之上额外满足：

- [ ] 该 Slice 全部 `Sn-Tx` 质量门 ✅。
- [ ] 该 Slice 设计 §16 验收标准逐条有测试证明。
- [ ] 跨 Slice 集成测试（前序 Slice 回归）全绿。
- [ ] `docs/rest-api.md`、`docs/openapi.yaml`、`docs/spec.md`（如需）同步。
- [ ] PR 描述含：测试计划、对照设计章节、`gitnexus_impact` 摘要。
- [ ] 合并后运行 `npx gitnexus analyze` 刷新索引（若已有 embeddings 则 `--embeddings`）。

### 0.7 Git 与分支

- 每个 Slice 起一条分支：`feat/workflow-slice-1`、`feat/workflow-slice-2`、`feat/workflow-slice-3`。
- 每个任务在分支上以小步提交，提交信息遵循 conventional commits（`feat:`/`test:`/`refactor:`/`chore:`），RED 与 GREEN 可分别提交以便回溯。
- 每个 Slice 一个 PR，PR 描述含测试计划、对照设计章节、`gitnexus_impact` 摘要、质量门结果。
- 合并门禁见 §0.6；合并后 `npx gitnexus analyze` 刷新索引。

### 0.8 质量检查命令速查

```bash
# 格式
cargo fmt --all -- --check
# Lint（零警告）
cargo clippy --workspace --all-targets -- -D warnings
# 全量测试
cargo test --workspace
# 覆盖率（任选其一）
cargo llvm-cov --workspace --html                # 需 cargo-llvm-cov
cargo tarpaulin --workspace --out Html           # 需 cargo-tarpaulin
# 影响与变更范围（GitNexus MCP）
#   gitnexus_impact({target, direction:"upstream"})
#   gitnexus_detect_changes({scope:"staged"})
```

---

## Slice 1 — Event-To-Action Core

> 设计章节：§3 架构、§4（WorkflowEvent/PromotionRule/WorkflowAction/WorkflowInstance）、§5 状态机、§8 失败处理、§9 数据模型、§10 事务并发、§13 Slice 1 API。
> 范围：事件落地 → 规则评估 → 幂等创建动作；落地一条规则「requirement publish 创建/更新 `work_item(kind=feature)`」。

### S1-T1 数据库迁移

- 新增 `migrations/014_workflow_automation.sql`，含表：
  - `workflow_events`（id, org_id, project_id, correlation_id, event_type, source_asset_id, payload jsonb, idempotency_key unique, created_at）
  - `promotion_rules`（id, org_id, scope enum, scope_ref, event_type, mutex_group, rule_version, priority, automation_level, filters jsonb, preconditions jsonb, action_template jsonb, max_cascade_depth, effective_from, effective_to, rollout_segment, enabled, created_at）
  - `workflow_instances`（id, org_id, project_id, correlation_id, template, status, lock_version, cascade_depth, created_at, updated_at）
  - `workflow_actions`（id, org_id, instance_id, action_type, target_asset_id, status, lock_version, idempotency_key unique, preconditions jsonb, postconditions jsonb, automation_level, compensation_* 字段先建占位, retry_count, next_retry_at, blocked_reason, created_at, updated_at）
- 索引：`workflow_events(correlation_id)`、`workflow_events(source_asset_id)`、`workflow_actions(instance_id)`、`workflow_actions(status, target_asset_id)`、`promotion_rules(scope, event_type, mutex_group, rule_version)`。
- 验收：`sqlx migrate run` 通过；`psql \d` 校验字段/约束。

### S1-T2 领域模型（adam-domain）

新增 `src/workflow/` 模块：

- `event.rs`：`WorkflowEvent`、`EventType` 枚举（`AssetPublished`/`DirtyResolved`/`PipelineFailed`…）、`CorrelationId` 值对象。
- `rule.rs`：`PromotionRule`、`RuleScope`(AssetType/Project/Organization)、`AutomationLevel`(Automatic/AgentSuggested/HumanApprovalRequired/HumanOnly)、`ActionTemplate`、`MutexGroup`。
- `action.rs`：`WorkflowAction`、`ActionType`、`BlockedReason` 枚举（按设计 §8）。
- `instance.rs`：`WorkflowInstance`、`WorkflowTemplate`。
- `state_machine.rs`：四个状态机的纯函数转换表 `fn transition(current, target) -> Result<Next, WorkflowError>`，覆盖设计 §5 全部矩阵；非法转换返回 `WorkflowError::IllegalTransition`。
- `conflict.rs`：规则冲突解析纯函数，严格实现设计 §8 第 516–524 条（scope→version→mutex→priority→automation_level→rule_id）。
- `idempotency.rs`：事件/动作幂等键构造（复用现有 `IdempotencyKey`）。
- `error.rs`：`WorkflowError`（`IllegalTransition`/`CascadeDepthExceeded`/`RuleConflict`/`PreconditionUnmet`/`ConcurrentModification`…）。

仓库 trait（追加到 `src/repository/mod.rs` 或新建 `src/repository/workflow.rs`）：
- `WorkflowEventRepository`：`append`（unique key 防重）、`find_by_correlation_id`、`find_by_asset`。
- `PromotionRuleRepository`：`find_enabled_for(event_type, scope, now)`、CRUD（admin 用，SERIALIZABLE）。
- `WorkflowInstanceRepository`：`create`、`find_by_id`、`update_cas`。
- `WorkflowActionRepository`：`create`、`find_by_id`、`find_by_instance`、`find_active_by_target`、`update_cas`。

在 `src/repository/in_memory.rs` 实现全部，保证可单测。

### S1-T3 应用服务（adam-application）

`src/services/workflow/`：

- `event_service.rs`（`WorkflowEventService`）：`append_event`——在单事务内写事件 + 触发评估；唯一键冲突时重载既有记录返回。
- `rule_evaluator.rs`（`PromotionRuleEvaluator`）：
  - 加载 enabled 规则（一致性快照，事务内）；
  - 过滤 + precondition 校验；
  - 调用 `conflict::resolve` 选出生效规则；
  - 幂等创建 `WorkflowAction`（unique key 冲突→重载）；
  - 级联深度校验 `cascade_depth < max_cascade_depth`，超限记 `CascadeDepthExceeded`；
  - 支持 dry-run / audit-only（不抑制 active 规则，但全部记录日志）。
- `instance_service.rs`（`WorkflowInstanceService`）：创建/推进实例（Saga 协调器骨架，本 Slice 仅单动作场景）。
- `action_service.rs`（`WorkflowActionService`）：状态转换 + precondition/postcondition 校验 + 发动作结果事件。

事务边界（设计 §10）：
- 事件处理事务内：写事件 → 评估规则（一致性快照）→ 创建动作，单 `BEGIN/COMMIT`。
- 实例/动作转换：行级锁 + `lock_version` CAS；admin 规则替换用 `SERIALIZABLE`。
- 幂等兜底：unique key 违约 → reload 既有行。

### S1-T4 种子规则与首条规则

- 在迁移或 `014` 附带 seed 中插入一条 `PromotionRule`：`event_type=AssetPublished, asset_type=requirement` → `ActionType=UpsertWorkItem(kind=feature)`，`automation_level=Automatic`，`scope=Organization`。
- `ActionTemplate` 携带从 requirement payload 映射到 work_item 字段的提取规则（保持显式、可测，设计 §8 第 396 条）。

### S1-T5 REST / MCP 适配（adam-adapters）

REST（`src/rest/workflow.rs`，按设计 §13 Slice 1）：
- `GET /api/workflow/events?project_id=&correlation_id=&asset_id=`
- `GET /api/workflow/instances/{workflow_instance_id}`
- `GET /api/workflow/actions?project_id=&status=&target_asset_id=`
- `POST /api/workflow/events`（要求 `Idempotency-Key` 头）

MCP（`src/mcp/workflow.rs`）：
- `query_workflow_state(workflow_instance_id)`

同步更新 `docs/rest-api.md` 与 `docs/openapi.yaml`（设计 §13 强制：Slice 未更新文档不算完成）。

### S1-T6 装配（adam-server）

`src/main.rs`：构造 Postgres 仓库实现 → 注入 services → 挂载 REST 路由 → 注册 MCP 工具。

### S1-T7 测试

- 单元：状态机全部转换矩阵、冲突解析 6 条规则、幂等键、级联深度超限。
- 应用：mockall 模拟仓库，验证「同一事件重放不产生重复动作」「并发事件不产生重复 active 动作」。
- 集成：Postgres 真实库验证 unique 约束、CAS 冲突、事件→动作闭环。
- E2E：`POST /api/workflow/events` 发布 requirement → 查询得到对应 work_item 动作。

### S1 验收标准（对照设计 §16 Slice 1）

1. 状态转换在 service 层显式校验，非法转换返回领域错误且不发事件。
2. 同一事件重放不创建重复 active 动作（unique key + reload）。
3. `AssetPublished(requirement)` 创建/更新 `work_item(kind=feature)` 动作。
4. 事件按 correlation_id 可重建链路。
5. 重叠规则按 scope/version/mutex/priority/automation_level 确定性解析。
6. 级联超限记 `CascadeDepthExceeded` 且不创建动作。
7. 文档化并验证事务隔离、锁策略、唯一约束（写入 ADR 或迁移注释）。
8. REST 文档与 OpenAPI 同步更新。

---

## Slice 2 — Agent Task Execution

> 设计章节：§4 AgentTask、§5 AgentTask 状态机、§10 claim 原子性、§13 Slice 2、§14 MCP。
> 范围：为 agent 可执行动作创建任务、原子认领、超时、结果回写、产出生成资产回链。

### S2-T1 迁移增量

- 在 `014` 中已建 `agent_tasks` 表（id, org_id, project_id, action_id, capability, status, agent_id, claimed_at, expires_at, result_payload jsonb, produced_asset_ids jsonb, lock_version, idempotency_key unique, created_at, updated_at）。
- 索引：`agent_tasks(status, capability)`、`agent_tasks(action_id)`。

### S2-T2 领域模型

- `src/workflow/agent_task.rs`：`AgentTask`、`TaskStatus`(Queued/Claimed/Running/Succeeded/Failed/Cancelled/Expired)、`Capability`、`TaskResult`。
- 状态机转换表并入 `state_machine.rs`（claim 原子 Queued→Claimed）。
- `AgentTaskRepository` trait：`create`、`list_queued(capability, project_id)`、`claim_cas(task_id, agent_id, expires_at)`（`SELECT … FOR UPDATE SKIP LOCKED` 等价语义在 service/infra 层实现）、`submit_result_cas`、`find_by_action`、`expire`。

### S2-T3 应用服务

`src/services/workflow/agent_task_service.rs`：
- `create_task_for_action`：ready 的 agent-executable 动作 → 创建 `AgentTask`。
- `claim_task`：原子 CAS `Queued→Claimed`，返回执行所需 context（复用 `VirtualInstance` 构造逻辑，见 `adam-domain/src/virtual_instance.rs`）。
- `submit_result`：CAS 存结果、链接 `produced_asset_ids`、发动作结果事件（驱动 `WorkflowAction` 进入 `Succeeded`）。
- `timeout_expired`：后台任务扫描 `expires_at`，按重试策略 fail 或释放父动作。

### S2-T4 首条 agent 路径

- ready 的 `work_item(kind=feature)` 动作 → 创建 `AgentTask(create_virtual_asset_context)`。
- 结果回写链接产出的 virtual instance / 资产到原 action。

### S2-T5 REST / MCP

REST（设计 §13 Slice 2）：
- `GET /api/agent-tasks?project_id=&status=queued&capability=`
- `POST /api/agent-tasks/{task_id}/claim`
- `POST /api/agent-tasks/{task_id}/result`

MCP（设计 §14）：
- `list_pending_agent_tasks(project_id, capability_filter)`
- `claim_agent_task(task_id, agent_id)`
- `submit_agent_task_result(task_id, result_payload, produced_asset_ids)`

更新 `docs/rest-api.md`、`docs/openapi.yaml`。

### S2-T6 后台 worker

- `adam-server` 起一个 `tokio::spawn` 周期任务扫描过期 task（间隔配置化）；本 Slice 可用简单轮询，后续可换队列。

### S2-T7 测试

- 单元：AgentTask 状态机、claim CAS 语义（并发两 claim 只一成功）。
- 集成：Postgres `SKIP LOCKED` 验证并发 claim；超时扫描。
- E2E：claim → submit_result → 动作 Succeeded → 产出资产回链。

### S2 验收标准（对照设计 §16 Slice 2）

1. claim 原子（`Queued→Claimed`），并发安全。
2. 结果回写链接产出资产并驱动动作完成。
3. 超时按策略 fail/释放父动作。
4. MCP 三工具幂等可用。
5. 文档同步。

---

## Slice 3 — Blocking, Approval, Compensation, Dead-Letter

> 设计章节：§4 ApprovalGate、§8 Saga/补偿/死信、§9 dead_letters 表、§13 Slice 3、§16 Slice 3。
> 范围：Dirty 阻塞、审批门、Saga 补偿、死信队列运维。

### S3-T1 迁移增量

- `014` 中已含 `approval_gates`（id, action_id, approver_type, approver_ref, status, decision_payload jsonb, deadline, decided_by, decided_at, lock_version, created_at）与 `workflow_dead_letters`（id, org_id, project_id, source_type, source_id, reason, context jsonb, status enum, created_at, resolved_at）。
- 死信状态：`Open/Assigned/Replayed/Resolved/Ignored`（设计 §9）。

### S3-T2 领域模型

- `src/workflow/approval_gate.rs`：`ApprovalGate`、`ApproverType`(Role/User/Group)、`GateStatus`、`GateDecision`。
- `src/workflow/dead_letter.rs`：`DeadLetter`、`DeadLetterStatus`、`DeadLetterSource`。
- `src/workflow/compensation.rs`：`CompensationPolicy`(None/BestEffort/RequiredBeforeFail/ManualOnly)、`CompensationAction` 声明。
- 状态机并入 `state_machine.rs`（ApprovalGate）。
- 仓库 trait：`ApprovalGateRepository`、`DeadLetterRepository`。

### S3-T3 应用服务

- `approval_gate_service.rs`：`request_approval`、`record_decision`（CAS `Pending→Approved/Rejected/Expired/Cancelled`）、解锁/失败等待动作。
- `compensation_service.rs`（Saga 协调）：
  - `WorkflowInstance` 为协调器；
  - 必需动作失败且已有副作用 → 调度 `compensation_action`；
  - `RequiredBeforeFail`：先补偿再 `Failed`；`BestEffort`：尽力补偿；
  - `ManualOnly`：进 `Blocked` + `WaitingManualIntervention`；
  - **不删除/改写已发布资产**，补偿以新纠正资产/状态转换/后续动作表达（设计 §8 第 506 条）；
  - 可恢复：进程重启后 reload 非终态实例继续（设计 §8 第 507 条）。
- `dead_letter_service.rs`：`list`/`replay`/`resolve`/`ignore`（补 `ignore` 端点——见审查遗留点 1）。
- 阻塞路径：Dirty 依赖阻塞 `publish_asset`；`DirtyResolved` 事件重评估并解锁动作。

### S3-T4 REST / MCP

REST（设计 §13 Slice 3，补 ignore）：
- `GET /api/approval-gates?project_id=&status=pending`
- `GET /api/approval-gates/{gate_id}`（审查遗留点 3，补详情）
- `POST /api/approval-gates/{gate_id}/approve`
- `POST /api/approval-gates/{gate_id}/reject`
- `POST /api/workflow/actions/{action_id}/retry`
- `POST /api/workflow/actions/{action_id}/cancel`
- `GET /api/workflow/dead-letters?project_id=&status=open`
- `POST /api/workflow/dead-letters/{dead_letter_id}/replay`
- `POST /api/workflow/dead-letters/{dead_letter_id}/resolve`
- `POST /api/workflow/dead-letters/{dead_letter_id}/ignore`（补）

更新 `docs/rest-api.md`、`docs/openapi.yaml`。

### S3-T5 权限与可观测性

- 权限（设计 §12）：REST/MCP 强制 project 成员、org 范围、approver 授权；人工动作在 operator 查询可见。
- 可观测性（设计 §12 Observability）：结构化日志/计数器覆盖规则评估、动作生命周期、claim 延迟、审批等待、级联超限、补偿、死信。

### S3-T6 测试

- 单元：ApprovalGate 状态机、补偿策略四分支、死信状态流转。
- 集成：Dirty 阻塞→`DirtyResolved` 解锁闭环；Saga 多动作部分失败→补偿调度；死信 replay/resolve/ignore。
- E2E：pipeline 失败→bugfix 审批→死信兜底。

### S3 验收标准（对照设计 §16 Slice 3）

1. 必需依赖 Dirty → 动作 `Blocked`。
2. `DirtyResolved` 重评估并解锁。
3. 审批门记录 approver/decision/deadline。
4. 每次自动转换发工作流事件。
5. correlation_id 重建从 requirement publish 到 agent task result 全链路。
6. 非可补偿失败 → blocked/dead-letter 含足够上下文。
7. 先前有副作用且后续必需失败 → 调度补偿。
8. 死信可 list/replay/resolve/ignore。
9. 每个 REST 端点同步更新 REST 文档与 OpenAPI。

---

## 里程碑与依赖

| 里程碑 | 内容 | 依赖 | 预估（人日） |
| --- | --- | --- | --- |
| M0 | 迁移 014 + 领域模型骨架 + in-memory 仓库 | 无 | 3 |
| M1 | Slice 1 全部（评估器/冲突/幂等/REST/MCP/文档） | M0 | 5 |
| M2 | Slice 2（AgentTask/claim/timeout/MCP） | M1 | 4 |
| M3 | Slice 3（阻塞/审批/Saga/死信/权限/可观测） | M2 | 6 |
| M4 | E2E 联调 + 文档定稿 + 索引更新 | M3 | 2 |

> 预估为粗估，按实际节奏调整。每个里程碑结束即合 PR。

## 风险与缓解

| 风险 | 缓解 |
| --- | --- |
| Postgres `SKIP LOCKED` / CAS 在 sqlx 下语义偏差 | 集成测试压测并发 claim；必要时降级为条件 `UPDATE … WHERE status='Queued'` |
| Saga 补偿跨资产难验证 | 先实现 `ManualOnly` 与 `BestEffort`，`RequiredBeforeFail` 用足测试矩阵 |
| 规则冲突解析歧义 | 纯函数 + 表驱动测试覆盖 6 条优先级 |
| 事件重放导致重复副作用 | unique key 兜底 + service 层 reload；E2E 重放测试 |
| 迁移 014 影响既有表 | 仅新增表/索引，不改既有迁移；CI 跑全量 migrate |

## 文档同步清单（每个 Slice 必做）

- [ ] `docs/rest-api.md` 增补对应端点
- [ ] `docs/openapi.yaml` 增补对应 paths/schemas
- [ ] `docs/spec.md` 如有语义变更则补注（尽量不改既有需求编号）
- [ ] 本计划勾选完成项
- [ ] 提交后 `npx gitnexus analyze` 刷新索引

## 验收后开放问题（设计 §17，实现时决策）

1. Agent task pull-only（已由设计隐含），push 推迟到后续版本——在实现时写明。
2. 默认需人工审批的动作集——M3 起界定。
3. pipeline run 是否可成为 asset instance——维持 Non-Goal，留待后续设计。
4. UI 可见指标 vs 仅日志——M3 与产品对齐。
