# Workflow Automation Implementation Progress

> 计划：[`2026-06-18-workflow-automation-implementation-plan.md`](./2026-06-18-workflow-automation-implementation-plan.md)
> 设计：[`2026-06-15-asset-driven-workflow-automation-design.md`](./2026-06-15-asset-driven-workflow-automation-design.md)
> 模式：TDD（RED→GREEN→REFACTOR→GATE），每任务完成过 §0.5 质量门。

## 进度总览

| 里程碑 | 任务 | 状态 | 质量门 |
| --- | --- | --- | --- |
| M0 | 领域骨架 + 迁移 + in-memory 仓库 | ✅ 完成 | ✅ 通过 |
| M1 | Slice 1 事件→动作核心 | ✅ 完成（T1–T7） | ✅ 通过 |
| M2 | Slice 2 Agent 任务执行 | ✅ 完成（T1–T7 + 复审 6 项修复） | ✅ 通过 |
| M3 | Slice 3 阻塞/审批/Saga/死信 | 进行中（应用服务层启动） | ✅ 当前服务层通过 |

## 环境备注

- Windows 下 Git Bash 的 `/usr/bin/link`（GNU coreutils）会遮蔽 MSVC `link.exe`，导致 `cargo test` 链接失败（`link: missing operand`）。这是既有环境问题，与本任务代码无关。
- 解决办法：测试需在 MSVC 开发环境下运行，即先 `call vcvars64.bat` 再执行 `cargo`（`rust-analyzer`/IDE 不受影响）。
- `cargo check`/`cargo fmt`/`cargo clippy`（仅解析）不受此问题影响。

## 详细日志

### M0 — 领域骨架 + 迁移 + in-memory 仓库 ✅

#### S1-T1 数据库迁移 `014_workflow_automation.sql`
- 新增表：`workflow_events`、`promotion_rules`、`workflow_instances`、`workflow_actions`、`agent_tasks`、`approval_gates`、`workflow_dead_letters`。
- 索引：幂等唯一键、correlation_id、source_asset、instance、status+target、mutex lookup、status+capability。
- 约束：枚举 CHECK 覆盖状态/automation_level/compensation_policy/blocked_reason/approver_type/dead_letter_source/status。
- 种子：为每个 organization 幂等插入首条规则 `asset_published(requirement) -> upsert_work_item(feature)`。
- TDD 锚点：纯迁移，锚点为表/约束存在性断言（待 infra 集成测试覆盖）。
> 质量门：✅ 迁移文件就绪；运行时校验由 Slice 1 集成测试承接。

#### 领域模型 `crates/adam-domain/src/workflow/`
- `error.rs`：`WorkflowError`（IllegalTransition/RuleConflict/CascadeDepthExceeded/PreconditionUnmet/ConcurrentModification/NotFound/DuplicateIdempotencyKey/Blocked/ValidationError）。
- `state_machine.rs`：`StateMachine` trait + 四状态机（InstanceStatus/ActionStatus/AgentTaskStatus/GateStatus），含 `can_transition_to`/`validate_transition`/终态判定；覆盖设计 §5 全部转换矩阵。
- `event.rs`：`WorkflowEvent`、`EventType`、`CorrelationId`、`WorkflowEventId`。
- `rule.rs`：`PromotionRule`、`RuleScope`、`AutomationLevel`、`MutexGroup`、`ActionType`、`ActionTemplate`、`is_effective_at`。
- `action.rs`：`WorkflowAction`、`BlockedReason`、`CompensationPolicy`、`CreateActionCommand`。
- `instance.rs`：`WorkflowInstance`、`WorkflowTemplate`、`CreateInstanceCommand`。
- `agent_task.rs`：`AgentTask`、`Capability`、`CreateAgentTaskCommand`。
- `approval_gate.rs`：`ApprovalGate`、`ApproverType`、`CreateApprovalGateCommand`。
- `dead_letter.rs`：`DeadLetter`、`DeadLetterSource`、`DeadLetterStatus`。
- `idempotency.rs`：事件/动作/实例动作/agent 任务幂等键构造（设计 §9）。
- `conflict.rs`：规则冲突解析纯函数，严格实现设计 §8 七条优先级（scope→version→mutex→priority→automation_level→rule_id）；dry-run/audit-only 不参与互斥且永不胜出但仍记录。

#### 仓库 trait + in-memory 实现
- `repository.rs`：六个仓库 trait（Event/Rule/Instance/Action/AgentTask/ApprovalGate/DeadLetter），写操作 CAS + `RepositoryError::ConcurrentModification`，幂等键 unique 兜底，含 `Arc<T>` blanket impl。
- `in_memory.rs`：全 trait 的内存实现 + 单测，验证幂等去重、CAS 冲突检测、agent task 原子 claim、dead letter 状态流转。

#### 质量门结果（§0.5）
- [x] `cargo fmt --all -- --check`：clean
- [x] `cargo clippy -p adam-domain --all-targets -- -D warnings`：clean（附带修复 `auth.rs` 测试中 Copy 类型的冗余 `.clone()`，为既有问题）
- [x] `cargo test -p adam-domain`：135 单测 + 全部集成测试通过（含 29 个 workflow 测试）
- [x] 文件/函数行数、不可变模式、`thiserror`/`#[async_trait]` 风格对齐既有 crate
- [x] 无生产 `unwrap`；幂等键与事务边界符合设计 §9/§10
- 备注：GitNexus impact 检查待提交后补充（M0 为新增模块，无既有符号被修改）

> 质量门：✅ 通过

---

### M1 — Slice 1 事件→动作核心（进行中）

#### S1-T3 应用服务 `crates/adam-application/src/services/workflow/` ✅

新增四个泛型服务（依赖注入经 `Arc<Repo>` + 可注入 `Clock`，对齐既有 `asset_lifecycle` 风格）：

- `event_service.rs`：`WorkflowEventService::append_event` —— 用 `event_idempotency_key` 派生键，先查既有键 fast-path 返回，写时遇 `DuplicateIdempotencyKey` reload 既有事件。覆盖验收标准 2（重放不产生重复）。
- `rule_evaluator.rs`：`PromotionRuleEvaluator::evaluate` —— 加载 enabled 规则 → 按 `source_asset_type_id` 与顶层 `filters` 精确匹配过滤 → `resolve_conflicts` 选 winner → 逐 winner 建 `WorkflowInstance`（Pending→Ready）+ 幂等建 `WorkflowAction`（unique key 冲突 reload，标 `reused`）→ `cascade_depth >= max_cascade_depth` 记 `CascadeViolation` 不建动作。覆盖验收标准 2/3/5/6。
- `instance_service.rs`：`WorkflowInstanceService` —— create / advance（`StateMachine::validate_transition` CAS，非法转换返 `IllegalTransition` 且不发事件）/ complete / fail（emit `ActionSucceeded`/`ActionFailed` 事件共享 correlation_id）。覆盖验收标准 1/4。
- `action_service.rs`：`WorkflowActionService::transition` —— CAS 转换 + `Succeeded` 时校验 `postconditions`（不满足返 `ConditionUnmet` 且不发事件）+ 终态结果 emit 结果事件。覆盖验收标准 1/4。
- `mod.rs`：共享 `Clock`/`SystemClock`/`ClockRef` 时间端口，便于测试注入。

事务边界（设计 §10）：当前为顺序调用；Postgres 装配时（S1-T6）将在单一 `BEGIN/COMMIT` 内完成「事件写入→规则评估→动作创建」。幂等兜底：unique key 违约 → reload。

TDD：每个服务先写失败测试（重放复用、级联超限不发动作、互斥组单 winner、payload 过滤、非法转换不发事件、postcondition 不满足阻断）再实现。

#### 质量门结果（§0.5）
- [x] `cargo fmt --all -- --check`：clean
- [x] `cargo clippy --workspace --all-targets -- -D warnings`：clean（附带修复 `adam-infrastructure/postgres.rs` 既有重复 `#[cfg(test)]` 属性，非本任务代码）
- [x] 测试编译通过：`cargo test -p adam-application` 的 lib/test 编译无错；仅链接步骤受既有 Git Bash `link.exe` 遮蔽问题阻断（需 `vcvars64.bat` 后执行，见环境备注）。测试体类型检查随 clippy 全量通过。
- [x] 文件/函数行数：单文件均 ≤ 400 行（event_service 212 / rule_evaluator 256 / rule_matching 99 / rule_evaluator_tests 182 / instance_service 298 / action_service 354）；函数 ≤ 50 行；嵌套 ≤ 4
- [x] 无生产 `unwrap`（仅测试）；不可变模式、`thiserror`、泛型 `Arc<Repo>` 风格对齐既有 crate
- [x] 幂等键与事务边界符合设计 §9/§10
- [x] 未引入设计 §18 Non-Goals 范畴
- [ ] GitNexus impact：M1 为新增服务模块，未改动既有符号；提交后 `gitnexus_detect_changes()` 复核
- [ ] 集成/E2E（S1-T7，含真实 Postgres 与 REST/MCP，S1-T5/T6 完成后承接）

> 质量门：✅ 通过（应用服务层；集成与适配层待 S1-T5/T6/T7）

#### S1-T5 REST / MCP 适配 ✅

- REST（`crates/adam-adapters/src/rest/workflow.rs`，新文件）：`GET/POST /api/workflow/events`、`GET /api/workflow/instances/{id}`、`GET /api/workflow/actions`；`POST events` 要求 `Idempotency-Key` 头，内部串联 `WorkflowEventService.append_event` → `PromotionRuleEvaluator.evaluate`，返回事件 + 创建动作 id + 级联超限。`AppState` 追加四个 workflow 仓库字段，`create_router` 注册路由。
- MCP（`crates/adam-adapters/src/mcp/mod.rs`）：新增 `query_workflow_state(workflow_instance_id)` 工具，返回实例状态 + 其动作列表；`McpServerState` 追加四个 workflow 仓库字段，`list_tools`/`call_tool` 注册分发。
- 文档：`docs/rest-api.md` 增补 Workflow Automation 节；`docs/openapi.yaml` 增补 `/api/workflow/events`、`/api/workflow/instances/{id}`、`/api/workflow/actions` paths 与 schemas。

#### S1-T6 装配 + Postgres 仓库 ✅

- `crates/adam-infrastructure/src/repositories/workflow_postgres.rs`（新文件，~850 行）：四个 workflow 仓库的 Postgres 实现，CAS + 唯一键幂等（23505 → `DuplicateIdempotencyKey`），行映射覆盖全部字段。`repositories/mod.rs` 注册并 re-export。
- `crates/adam-server/src/main.rs`：`Repositories` 追加四个 workflow 仓库，memory/postgres 两后端均装配；`rest_state`/`mcp_state` 注入。

#### S1-T7 集成 / E2E ✅

- `rest/mod.rs` 测试模块新增 E2E：seed 规则 → `POST /api/workflow/events`（asset_published）→ 断言 201 + 1 动作 → 重放同事件复用同一动作（验收标准 2）→ `GET /api/workflow/events?correlation_id=` 取回事件（验收标准 4）。
- 真实 Postgres 集成测试（unique 约束/CAS/事件→动作闭环）受既有 link.exe 环境阻断，待 MSVC 环境执行；PG 仓库逻辑与 in-memory 对齐，由 S1-T3 服务层测试与上述 E2E 间接覆盖。

#### Slice 1 质量门结果（§0.5/§0.6）
- [x] `cargo fmt --all -- --check`：clean
- [x] `cargo clippy --workspace --all-targets -- -D warnings`：clean
- [x] 测试编译通过（lib+test 随 clippy 全量类型检查）；仅链接步骤受既有 Git Bash `link.exe` 遮蔽阻断（见环境备注），需 MSVC 环境跑 `cargo test`
- [x] S1 全部任务（T1–T7）质量门 ✅；验收标准 1–8（§16）均有对应实现/测试
- [ ] 提交后 `gitnexus_impact`/`detect_changes` 复核；`npx gitnexus analyze` 刷新索引
- [ ] PG 集成测试在 MSVC 环境补跑

> 质量门：✅ Slice 1 通过（待 MSVC 环境跑 `cargo test` 验证链接后全绿 + PG 集成补跑）

#### Slice 1 代码审查修复（P0/P1/P2）

- **[P0] 孤儿实例**：`rule_evaluator` 在动作幂等键冲突（并发竞态）分支新增「取消本 worker 刚创建的 Ready 实例」回滚，reload 当前 lock_version 后 CAS 置 `Cancelled`，杜绝无动作的孤儿 Ready 实例。新增测试 `concurrent_duplicate_action_cancels_orphan_instance` 验证仅留 winner 实例非终态。
- **[P1] 关联链重建**：`action_service` 注入 `WorkflowInstanceRepository`，结果事件 `ActionSucceeded/ActionFailed` 改用父实例的 `correlation_id`（而非 action id 派生），保证 correlation_id 可重建全链路；不可解析时降级到 action id 派生（best-effort，不丢事件）。测试改为按父实例 correlation_id 取回结果事件。
- **[P1] REST 组织边界**：`list_events`/`get_instance`/`list_actions` 全部按 `auth.principal.organization_id` 过滤，跨组织查询返回 404/空。新增 E2E `get_workflow_instance_cross_org_returns_404`。
- **[P1] MCP 授权**：`query_workflow_state` 增加组织边界校验，跨组织实例返回 not-found。
- **[P2] actions 查询语义**：文档化「需 `target_asset_id` 定位、`project_id` 因无实例 join 暂无法精确过滤，Slice 2 补 instance-scoped 查询」。
- **[P2] Postgres 集成测试**：新增 `crates/adam-infrastructure/tests/workflow_postgres.rs`（`#[ignore]`，CI 用 `DATABASE_URL` 显式跑），覆盖事件/动作唯一键约束、实例 CAS 冲突。

#### 复审后质量门
- [x] `cargo fmt --all -- --check`：clean
- [x] `cargo clippy --workspace --all-targets -- -D warnings`：clean
- [x] 测试体随 clippy 全量类型检查通过（含新增 P0/P1/跨组织测试）；链接步骤仍受既有 link.exe 遮蔽阻断（见环境备注）
- [x] 审查 P0/P1/P2 全部有对应修复与测试
- [ ] MSVC 环境跑 `cargo test` 全绿；CI 跑 `--ignored` PG 集成

> 质量门：✅ Slice 1 复审修复通过（P0/P1/P2 已闭环；待 MSVC+CI 验证链接与 PG 集成）

---

### M2 — Slice 2 AgentTask / claim / timeout / MCP（进行中）

#### S2-T2/T3 领域仓库 + 应用服务 ✅

- `AgentTaskRepository` 补 `find_by_action`，in-memory 与 Postgres 实现保持一致；Postgres `PostgresAgentTaskRepository` 覆盖 create/find/list_queued/claim_cas/update_cas/find_expired/find_by_status。
- `AgentTaskService` 新增 create/claim/submit/timeout 路径：ready action 幂等创建任务；claim 使用 CAS 从 `queued` 到 `claimed` 并写入 lease；result 写回 payload/produced assets，并驱动父 action 到 `succeeded`；timeout 将过期租约置 `expired`。
- 单测覆盖幂等创建、并发 claim 只有一次成功、result 回写并完成父 action、过期扫描。

#### S2-T5 REST / MCP 适配 ✅

- REST 新增 `GET /api/agent-tasks`、`POST /api/agent-tasks/{task_id}/claim`、`POST /api/agent-tasks/{task_id}/result`；均按 caller organization 边界过滤。
- MCP 新增 `list_pending_agent_tasks`、`claim_agent_task`、`submit_agent_task_result`，复用同一个 `AgentTaskService`。
- E2E 覆盖 REST list→claim→result、MCP list→claim→submit。

#### S2-T6 背景过期扫描 ✅

- `adam-server` 启动 REST 时挂载 AgentTask expiry worker，默认每 60 秒执行 `AgentTaskService::timeout_expired(Utc::now())`。
- `ADAM_AGENT_TASK_EXPIRY_INTERVAL_SECONDS=0` 可关闭 worker。

#### 当前质量门

- [x] `cargo check -p adam-server -p adam-adapters`
- [x] `cargo test -p adam-adapters agent_task -- --nocapture`
- [x] `cargo test -p adam-application services::workflow::agent_task_service -- --nocapture`
- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --workspace --all-targets -- -D warnings`
- [x] `cargo test --workspace`：全部通过；Postgres 集成测试按 `#[ignore]` 跳过
- [x] `gitnexus_detect_changes(scope="all")`：已复核；因当前工作区包含 Slice 1 大量未提交变更和共享入口结构变动，整体风险显示 CRITICAL，直接验证由 fmt/clippy/workspace test 覆盖

#### Slice 2 复审修复（6 项，对照审查 P0–P3）

- **[P0] timeout_expired 父动作处理**：`timeout_expired` 不再仅置 task=Expired。新增 `fail_parent_action`：当 `retry_count >= max_retries` 时驱动父动作 `Ready→InProgress→Failed`（发 `ActionFailed` 结果事件，correlation_id 沿用父实例），重试预算未耗尽则保留 `Ready` 待重排；task CAS 冲突（agent 抢先提交结果）跳过该项继续批处理；单条动作转换失败不中止整批。新增测试 `timeout_expired_fails_parent_action_when_retries_exhausted`、`timeout_expired_leaves_action_when_retries_remain`。
- **[P1] submit_result 操作顺序**：先驱动父动作 `Succeeded` 再 CAS task `Succeeded`。动作转换失败（非法转换/postcondition 不满足/CAS）时 task 保持 `Claimed/Running` 非终态，杜绝「终态 task 指向非终态 action」的不一致。新增测试 `submit_result_leaves_task_non_terminal_when_action_postcondition_fails`。
- **[P2] Postgres agent_task 集成测试**：`crates/adam-infrastructure/tests/workflow_postgres.rs` 新增 3 个 `#[ignore]` 测试——并发 claim 仅一成功（`tokio::join!` 双 claim）、`find_expired` 仅返回已过期租约且排除 queued、`update_cas` 检测 `ConcurrentModification`。CI 用 `DATABASE_URL` 跑 `--ignored`。
- **[P2] S2-T4 VirtualInstance 回链**：新增 `build_claim_context(&task)` 复用 `VirtualInstance::new`，以父动作 `target_asset_id` 为锚、`target_asset_type_id` 为类型构造上下文（无 target 类型时返回 `None`）；新增类型化 `ProducedAssets` 枚举（`None`/`VirtualInstance`/`Assets`）区分产出资产类型，`submit_result_typed` 经 `into_asset_ids` 落库。新增测试 `build_claim_context_reuses_virtual_instance_construction`、`submit_result_typed_links_virtual_instance_and_succeeds_action`、`build_claim_context_returns_none_without_target_asset_type`。
- **[P3] expiry worker 节流**：`spawn_agent_task_expiry_worker` 的 `tokio::time::interval` 设 `MissedTickBehavior::Skip`，避免扫描超时后补连发。
- **[P3] 死代码与常量**：删除 `workflow_postgres.rs` 末尾 `_pool_arc` 死函数及 `use std::sync::Arc`；REST/MCP 的 lease 默认 900s 提取为 `DEFAULT_AGENT_TASK_LEASE_SECONDS` 常量。

#### 环境备注更新

- 已确认 Git Bash `/usr/bin/link`（GNU coreutils）遮蔽 MSVC `link.exe` 的链接问题**可绕过**：用 `scripts/cargo-msvc.sh` 在 MSVC 环境下跑 `cargo`（`vcvars64.bat` 注入 `LIB/INCLUDE`，MSVC `link.exe` 前置 PATH）。该脚本已用于本 slice 的 `cargo clippy`/`cargo test --workspace` 全量验证，全部通过。

#### 复审后质量门

- [x] `cargo fmt --all -- --check`：clean
- [x] `cargo clippy --workspace --all-targets -- -D warnings`：clean（经 `scripts/cargo-msvc.sh` 在 MSVC 环境执行）
- [x] `cargo test --workspace`：全绿（domain 137 / application 67 / adapters 22+14 / infrastructure 8 / server 3 等；PG 集成 6 项 `#[ignore]` 跳过）
- [x] 审查 6 项 P0–P3 全部有对应实现与测试

> 质量门：✅ Slice 2 复审修复通过（6 项闭环；MSVC 环境下 fmt/clippy/test 全绿；CI 待跑 `--ignored` PG 集成）

---

<!-- 后续 Slice 在此追加 -->

### M3 — Slice 3 Blocking / Approval / Compensation / Dead-Letter（进行中）

#### S3-T3 应用服务首段 ✅

- `ApprovalGateService`：新增 `request_approval` 与 `record_decision`。`request_approval` 将 Ready action 推进到 `WaitingApproval` 并创建 pending gate；批准决策记录 approver/decision/deadline 相关字段并把父 action 解锁到 `Ready`；拒绝决策记录结果并把父 action 推进到 `Failed`；审批事件使用父 workflow instance 的 `correlation_id`，可重建链路。
- `DeadLetterService`：新增 `enqueue`、`list`、`replay`、`resolve`、`ignore`。`ignore` 闭合 Slice 3 审查遗留点；list 支持 organization + status，并可按 project 过滤；terminal dead-letter 不允许再次 replay。
- REST/MCP 适配尚未接入：Postgres workflow 仓库当前尚无 `ApprovalGateRepository` / `DeadLetterRepository` 实现，避免先把 REST 装到仅 memory 可用的半成品。下一步应先补 Postgres 仓库实现与 ignored 集成测试，再挂 REST/MCP 端点和文档。

#### 当前质量门

- [x] `cargo test -p adam-application services::workflow::approval_gate_service -- --nocapture`
- [x] `cargo test -p adam-application services::workflow::dead_letter_service -- --nocapture`
- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --workspace --all-targets -- -D warnings`
- [x] `cargo test --workspace`：全部通过；Postgres 集成测试按 `#[ignore]` 跳过
- [x] `gitnexus_impact`：`ApprovalGateRepository` / `DeadLetterRepository` 为 LOW；`create_router` 为 CRITICAL（REST 总入口，高影响，故本轮未挂 REST）
- [x] `gitnexus_detect_changes(scope="all")`：已复核；当前工作区包含 Slice 1/2 大量未提交变更，整体风险仍显示 CRITICAL

> 质量门：✅ Slice 3 应用服务首段通过；REST/MCP/Postgres 适配待下一小步。
