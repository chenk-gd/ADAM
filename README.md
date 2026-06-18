# ADAM — AI-Driven Asset Management

[![Rust](https://img.shields.io/badge/Rust-1.85+-orange.svg)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

ADAM 是一个面向 AI Agent 与研发团队的研发资产管理系统。它不替代 Git、Wiki、Jira、CI/CD 等系统存储原始内容，而是在这些系统之上建立统一的研发资产索引与关系层，将需求、工作项、设计文档、代码提交、测试用例、流水线等研发产物抽象为可查询、可版本化、可追踪状态、可参与工作流的资产。

ADAM 的核心目标是让 AI Agent 能够通过统一入口获取完整、准确且不过度冗余的研发上下文。现有 DevOps 平台通常已经在代码中心、工作项中心、测试中心和流水线中心之间存在业务交互，但向 Agent 暴露能力时往往仍按中心分别开放接口。Agent 可以读取单个中心的内容，却缺少统一的资产关系层来理解这些内容之间的依赖、影响和处理状态。ADAM 通过资产图、版本基线、Dirty 传播和工作流自动化，将分散事实关联起来，支撑 Agent 检索、推理和执行研发任务。

## 核心能力

- **研发资产索引**：统一登记需求、工作项、设计文档、代码提交、测试用例和流水线等资产，保留外部系统引用和结构化元数据。
- **依赖图与影响分析**：用严格 DAG 表达资产之间的上游依赖关系，支持关系类型、传播策略、上下游查询和影响分析。
- **版本与基线管理**：区分发布依赖快照和当前有效依赖基线，既支持历史审计，也支持当前 Dirty 判断和 Agent 上下文构造。
- **状态传播**：当上游资产发布新版本时，根据依赖关系和传播策略将直接下游标记为 Dirty，提示需要审查或重新发布。
- **Agent 上下文构造**：从资产、工作流动作或 AgentTask 出发，生成包含相关资产、关系、版本依据、状态依据和操作边界的结构化上下文。
- **资产驱动工作流**：通过事件、规则、动作、Agent 任务和审批门禁，将资产变化转化为可追踪、可审计、可重试的处理流程。
- **REST 与 MCP 接口**：面向平台管理、外部系统集成和 AI Agent 提供统一业务能力，所有入口遵循同一组领域规则。

## 核心概念

| 概念 | 说明 |
| --- | --- |
| AssetType | 资产类型，定义研发资产分类、元数据结构和允许参与的依赖规则 |
| AssetInstance | 资产实例，表示一个具体需求、工作项、设计文档、代码提交、测试用例或流水线记录 |
| AssetDependency | 资产依赖边，方向为下游资产 `source -> target` 上游资产 |
| AssetVersion | 资产发布历史，记录发布时的元数据和依赖快照 |
| DirtyQueue | Dirty 处理队列，记录上游变化对下游资产造成的待处理影响 |
| VirtualInstance | 面向 Agent 的临时上下文对象，用于组织一次查询或任务执行所需资产集合 |
| WorkflowEvent | 资产或外部系统产生的不可变事件，用于审计和触发规则 |
| WorkflowAction | 由规则生成的流程动作，可自动执行、交给 Agent、等待审批或进入人工处理 |
| AgentTask | 分配给 AI Agent 的可领取任务 |
| ApprovalGate | 需要人工审批或人工判断的决策点 |

## 资产类型

系统内置首批资产类型如下：

| 资产类型 | 说明 |
| --- | --- |
| `requirement` | 需求或缺陷范围描述 |
| `work_item` | 研发工作容器，通过 `metadata.work_item_kind` 区分子类型 |
| `design_doc` | 概要设计、详细设计、技术方案等 |
| `code_commit` | 代码提交或代码变更引用 |
| `test_case` | 测试用例、验证场景 |
| `pipeline` | 流水线定义或可执行流水线资产 |

`work_item` 使用元数据字段 `work_item_kind` 区分子类型。当前内置规则支持 `feature`、`bugfix` 和 `test_execution`。这样可以保持资产类型稳定，同时通过元数据表达不同工作语义。

## 依赖关系与传播策略

资产依赖方向固定为：

```text
下游资产 source -> 上游资产 target
```

关系类型用于表达业务语义，传播策略用于控制上游发布后是否影响下游状态。

| 关系 | 语义 | 默认传播策略 |
| --- | --- | --- |
| `depends_on` | 一般依赖 | Dirty |
| `implements` | 下游实现上游 | Dirty |
| `fixes` | 下游修复上游 | Dirty |
| `verifies` | 下游验证上游 | Dirty |
| `references` | 下游引用上游作为上下文 | ContextOnly |
| `executes` | 下游执行上游 | ContextOnly |
| `produces` | 下游产生上游或产物记录 | AuditOnly |
| `blocks` | 下游阻塞上游 | AuditOnly |
| `relates_to` | 通用关联 | AuditOnly |

传播策略包括：

- `Dirty`：上游发布新版本时，下游进入 Dirty。
- `ContextOnly`：仅用于上下文查询，不触发 Dirty。
- `AuditOnly`：仅用于审计和追溯，不触发 Dirty。

## 资产状态

| 状态 | 说明 |
| --- | --- |
| `Clean` | 当前资产可信，上游变化已同步或确认无影响 |
| `Dirty` | 至少一个上游资产发布了新版本，当前资产尚未处理 |
| `Archived` | 资产停止维护，不再发布，不接收 Dirty |
| `Final` | 不可变资产状态，如代码提交、流水线执行记录，创建后不变 |

只有发布新版本会触发下游 Dirty。手工 Clean、归档和状态刷新不会继续向下游传播。

## 工作流自动化

ADAM 将资产状态变化进一步转化为流程动作。资产图说明资产之间的依赖关系，Dirty 传播说明受影响范围；工作流自动化负责决定后续处理主体、处理内容和完成条件。

典型流程包括：

- 需求发布后创建或关联 `work_item(kind=feature)`。
- 上游资产变化后将下游标记为 Dirty，并创建审查或处理动作。
- 流水线失败后创建审批门禁，批准后生成 `work_item(kind=bugfix)`。
- WorkflowAction 创建 AgentTask，Agent 领取任务、提交结果并关联产出资产。
- 多次失败的动作进入 WorkflowDeadLetter，由运维或负责人重放、解决或忽略。

## Agent 上下文构造

ADAM 面向 Agent 提供结构化上下文，而不是简单返回资产列表。上下文构造以资产、WorkflowAction 或 AgentTask 为入口，结合权限边界、依赖图、版本基线、Dirty 状态、工作流状态和上下文预算，生成可供 Agent 消费的上下文包。

上下文包包含：

- 入口对象
- 相关资产集合
- 资产之间的关系类型、传播策略和方向
- 发布快照、当前有效基线和必要的最新版本信息
- Dirty 原因、失败原因、审批状态和阻塞状态
- Agent 可读取、可建议、可回写或必须等待审批的操作边界
- 上下文生成策略、过滤规则和来源引用

## 架构概览

![逻辑分层](docs/diagrams/04-02-logical-layers.svg)

所有入口都进入同一组应用服务和领域模型，避免 REST、MCP、Hook、Worker 等入口产生不一致的资产状态或工作流结果。

运行时组件关系如下：

![运行时组件](docs/diagrams/04-04-runtime-components.svg)

## 项目结构

这是一个 Rust workspace，主要 crate 如下：

| Crate | Path | 说明 |
| --- | --- | --- |
| `adam-domain` | `crates/adam-domain` | 核心领域模型、实体、规则和业务约束 |
| `adam-application` | `crates/adam-application` | 应用服务、用例编排、状态传播和工作流协调 |
| `adam-infrastructure` | `crates/adam-infrastructure` | PostgreSQL repository 和基础设施实现 |
| `adam-adapters` | `crates/adam-adapters` | REST API 与 MCP Server 适配层 |
| `adam-server` | `crates/adam-server` | 应用入口和运行配置 |

## 快速开始

### 环境要求

- Rust 1.85+
- PostgreSQL
- MCP-compatible client（可选，用于 AI Agent 集成）

### 构建

```bash
cargo build
cargo build --release
```

### 测试

```bash
cargo test
cargo test -- --nocapture
```

### 格式化与检查

```bash
cargo fmt
cargo check
cargo clippy
```

### 运行

```bash
cargo run --bin adam-server
```

示例配置：

```toml
[server]
host = "0.0.0.0"
port = 8080

[database]
url = "postgres://user:pass@localhost/adam"

[mcp]
enabled = true
server_name = "adam-mcp-server"
```

## 接口

### REST API

REST API 面向平台管理、外部系统集成和操作界面，覆盖资产类型管理、资产发布与查询、依赖图与影响分析、Dirty 处理、手工 Clean、工作流事件、动作、审批和死信处理。

参见 [docs/rest-api.md](docs/rest-api.md) 和 [docs/openapi.yaml](docs/openapi.yaml)。

### MCP Server

MCP 接口面向 AI Agent。Agent 可通过 MCP 查询资产上下文、创建虚拟上下文、发布资产、领取 AgentTask、提交任务结果和查询工作流状态。

## 文档

- [系统概要设计](docs/system-overview-design.md)
- [需求规格说明书](docs/spec.md)
- [REST API 文档](docs/rest-api.md)
- [OpenAPI 契约](docs/openapi.yaml)
- [Asset-Driven Workflow Automation 设计](docs/plans/2026-06-15-asset-driven-workflow-automation-design.md)
- [Work Item Kind Dependency Model 设计](docs/plans/2026-06-15-work-item-kind-dependency-model.md)
- [Version Constraints 说明](docs/version-constraints.md)
- [Jenkins 集成指南](docs/jenkins-integration-guide.md)

## 非目标

ADAM 当前不承担以下职责：

- 替代 Git、Wiki、Jira、CI/CD 等系统存储原始内容。
- 成为完整项目管理系统。
- 成为通用 BPMN 流程引擎。
- 自动合并代码或自动部署生产。
- 提供不受约束的 Agent 自主执行能力。

## License

MIT License — see [LICENSE](LICENSE) for details.
