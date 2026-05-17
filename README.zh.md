# ADAM — 研发资产管理系统

[![Rust](https://img.shields.io/badge/Rust-1.85+-orange.svg)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

**ADAM** (Asset Dependency and Management) 是一个研发资产管理平台，支持依赖追踪和生命周期管理。它为大型语言模型（LLM）提供上下文服务，结构化地管理需求、设计文档、代码提交、测试用例和流水线等研发资产。

---

## 系统概述

ADAM 提供统一平台，实现以下功能：

- **资产类型管理** — 使用 JSON Schema 元数据定义和管理各类研发资产类型
- **依赖关系追踪** — 基于有向无环图（DAG）维护资产间的依赖关系
- **生命周期管理** — 追踪资产状态（Clean、Dirty、Archived）并管理版本
- **LLM 上下文服务** — 通过 MCP 协议为 AI Agent 提供结构化资产上下文
- **CI/CD 集成** — 通过 Git Hooks 和流水线集成实现自动资产注册

## 系统架构

```
┌─────────────────────────────────────────────────────────────┐
│                    ADAM 平台                                 │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────┐   │
│  │  需求管理  │  │  工作项  │  │  设计文档  │  │  代码提交  │   │
│  └──────────┘  └──────────┘  └──────────┘  └──────────┘   │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐                  │
│  │  测试用例  │  │  流水线  │  │  其他资产  │                  │
│  └──────────┘  └──────────┘  └──────────┘                  │
│                                                             │
│  ┌─────────────────────────────────────────────────────┐   │
│  │              依赖关系引擎 & 状态管理                 │   │
│  └─────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
                              │
                    ┌─────────┴─────────┐
                    ▼                   ▼
            ┌─────────────┐     ┌─────────────┐
            │  MCP 服务   │     │  REST API   │
│            │  接口       │     │  接口       │
            └─────────────┘     └─────────────┘
```

## 项目结构

这是一个 Rust 工作空间项目，包含多个 crate：

| Crate | 路径 | 说明 |
|-------|------|------|
| `adam-domain` | `crates/adam-domain` | 核心领域模型、实体和业务逻辑 |
| `adam-application` | `crates/adam-application` | 应用服务、用例和状态传播 |
| `adam-infrastructure` | `crates/adam-infrastructure` | 数据库存储库和外部服务适配器 |
| `adam-adapters` | `crates/adam-adapters` | MCP 和 REST API 接口实现 |
| `adam-server` | `crates/adam-server` | 应用程序入口和配置 |

## 核心概念

### 资产类型
预定义和可扩展的资产类型：
- `requirement` — 功能性和非功能性需求
- `work_item` — 任务和开发工作项
- `design_doc` — 架构设计和详细设计文档
- `code_commit` — 代码变更和 Pull Request
- `test_case` — 测试场景和用例
- `pipeline` — CI/CD 流水线定义

### 资产状态
- **Clean** — 资产与上游依赖保持同步，内容可信
- **Dirty** — 上游资产已发布新版本，等待审查确认
- **Archived** — 资产已归档，只读状态，不再维护

### 依赖模型
- 严格 DAG（有向无环图）— 禁止循环依赖
- 声明版本 — 发布时的快照，用于历史追溯和审计
- 有效基线 — 当前上游版本，用于上下文查询和 Dirty 判断
- 状态传播 — 仅发布操作触发下游 Dirty 状态

## 快速开始

### 环境要求

- Rust 1.85+（2024 edition）
- PostgreSQL（用于持久化存储）
- MCP 兼容客户端（可选，用于 AI 集成）

### 构建

```bash
# Debug 构建
cargo build

# Release 构建
cargo build --release
```

### 运行测试

```bash
# 运行所有测试
cargo test

# 显示输出
cargo test -- --nocapture
```

### 运行应用

```bash
# 运行服务器
cargo run --bin adam-server

# 带配置运行
cargo run --bin adam-server -- --config config.toml
```

### 代码格式化与检查

```bash
# 格式化代码
cargo fmt

# 运行代码检查
cargo clippy

# 仅检查不编译
cargo check
```

## MCP 服务集成

ADAM 为 AI Agent 提供 MCP（Model Context Protocol）服务接口：

- `query_assets` — 按项目、类型或状态查询资产
- `create_virtual_asset` — 创建临时查询上下文
- `publish_asset` — 发布新版本的资产
- `get_asset_dependencies` — 获取资产依赖信息

## 配置示例

`config.toml` 示例：

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

## 文档

- [需求规格说明书](docs/spec.md) — 完整功能需求（中文）
- [MCP 接口指南](docs/mcp-guide.md) — MCP 服务使用说明
- [API 参考文档](docs/api.md) — REST API 接口文档

## 贡献指南

欢迎贡献代码！请确保：

1. 代码符合 Rust 习惯用法，通过 `cargo clippy` 检查
2. 新功能包含测试（覆盖率 80%+）
3. API 变更时更新文档

## 许可证

MIT 许可证 — 详见 [LICENSE](LICENSE) 文件。

---

<p align="center">
  <a href="./README.md">English Version</a>
</p>
