#!/usr/bin/env python3
"""Batch generate all ADAM system overview diagrams in Flat Icon style with Chinese font support."""

import json
import os
import subprocess
import sys

SCRIPT_DIR = os.path.dirname(__file__)
SKILL_DIR = os.path.expanduser("~/.claude/skills/fireworks-tech-graph")
GEN_SCRIPT = os.path.join(SKILL_DIR, "scripts", "generate-from-template.py")
OUTPUT_DIR = SCRIPT_DIR

os.makedirs(OUTPUT_DIR, exist_ok=True)

def generate(name, template, data):
    path = os.path.join(OUTPUT_DIR, f"{name}.svg")
    cmd = [sys.executable, GEN_SCRIPT, template, path, json.dumps(data, ensure_ascii=False)]
    result = subprocess.run(cmd, capture_output=True, text=True, encoding='utf-8', errors='replace')
    if result.returncode != 0:
        print(f"  X {name}: {result.stderr.strip() if result.stderr else 'unknown error'}")
        return False
    print(f"  OK {name}.svg")
    png_path = path.replace(".svg", ".png")
    try:
        with open(path, "rb") as f:
            svg_bytes = f.read()
        import cairosvg
        png = cairosvg.svg2png(bytestring=svg_bytes, output_width=1920)
        with open(png_path, "wb") as f:
            f.write(png)
        size = os.path.getsize(png_path)
        print(f"    OK {name}.png ({size} bytes)")
    except Exception as e:
        print(f"    X PNG: {e}")
    return True

STYLE = 1
FONT_OVERRIDE = {"font_family": "'Noto Sans SC', 'Helvetica Neue', Helvetica, Arial, 'Microsoft YaHei', sans-serif"}

# ========================================================================
print("Generating 04-01-system-context...")
generate("04-01-system-context", "architecture", {
    "title": "ADAM 系统上下文",
    "style": STYLE,
    "style_overrides": FONT_OVERRIDE,
    "viewBox": "0 0 960 600",
    "nodes": [
        {"id": "dev", "label": "研发人员", "x": 60, "y": 140, "width": 120, "height": 50},
        {"id": "agent", "label": "AI Agent / MCP", "x": 60, "y": 260, "width": 140, "height": 50},
        {"id": "git", "label": "Git 平台", "x": 60, "y": 400, "width": 120, "height": 50},
        {"id": "wiki", "label": "Wiki / 文档系统", "x": 220, "y": 420, "width": 140, "height": 50},
        {"id": "pm", "label": "项目管理系统", "x": 400, "y": 420, "width": 140, "height": 50},
        {"id": "ci", "label": "CI/CD 系统", "x": 580, "y": 420, "width": 120, "height": 50},
        {"id": "adam", "label": "ADAM", "kind": "double_rect", "x": 320, "y": 240, "width": 160, "height": 90, "accent_fill": "#7c3aed"},
        {"id": "db", "label": "PostgreSQL", "kind": "cylinder", "x": 680, "y": 180, "width": 130, "height": 80},
        {"id": "queue", "label": "异步 Worker / 调度器", "kind": "rect", "x": 680, "y": 300, "width": 150, "height": 60}
    ],
    "arrows": [
        {"source": "dev", "target": "adam", "flow": "control"},
        {"source": "agent", "target": "adam", "flow": "control"},
        {"source": "git", "target": "adam", "flow": "control"},
        {"source": "wiki", "target": "adam", "flow": "control"},
        {"source": "pm", "target": "adam", "flow": "control"},
        {"source": "ci", "target": "adam", "flow": "control"},
        {"source": "adam", "target": "git", "flow": "data"},
        {"source": "adam", "target": "wiki", "flow": "data"},
        {"source": "adam", "target": "pm", "flow": "data"},
        {"source": "adam", "target": "ci", "flow": "data"},
        {"source": "adam", "target": "db", "flow": "write"},
        {"source": "adam", "target": "queue", "flow": "async"}
    ],
    "legend": [
        {"flow": "control", "label": "控制/交互"},
        {"flow": "data", "label": "数据读写"},
        {"flow": "write", "label": "持久化存储"},
        {"flow": "async", "label": "异步任务"}
    ]
})

# ========================================================================
print("Generating 04-02-logical-layers...")
generate("04-02-logical-layers", "architecture", {
    "title": "ADAM 逻辑分层",
    "style": STYLE,
    "style_overrides": FONT_OVERRIDE,
    "viewBox": "0 0 960 680",
    "containers": [
        {"label": "接口层", "x": 60, "y": 120, "width": 840, "height": 90},
        {"label": "应用层", "x": 60, "y": 230, "width": 840, "height": 90},
        {"label": "领域层", "x": 60, "y": 340, "width": 840, "height": 90},
        {"label": "基础设施层", "x": 60, "y": 450, "width": 840, "height": 90}
    ],
    "nodes": [
        {"id": "rest", "label": "REST API", "x": 120, "y": 150, "width": 130, "height": 50},
        {"id": "mcp", "label": "MCP Server", "x": 300, "y": 150, "width": 130, "height": 50},
        {"id": "hooks", "label": "Git Hook / Webhook / CI Adapter", "x": 480, "y": 150, "width": 280, "height": 50},
        {"id": "assetsvc", "label": "资产服务", "x": 100, "y": 260, "width": 120, "height": 50},
        {"id": "versionsvc", "label": "版本发布服务", "x": 250, "y": 260, "width": 140, "height": 50},
        {"id": "depsvc", "label": "依赖与影响分析服务", "x": 420, "y": 260, "width": 170, "height": 50},
        {"id": "statesvc", "label": "状态传播服务", "x": 620, "y": 260, "width": 140, "height": 50},
        {"id": "wfsvc", "label": "工作流服务", "x": 780, "y": 260, "width": 120, "height": 50},
        {"id": "assetmodel", "label": "资产模型", "x": 100, "y": 370, "width": 120, "height": 50},
        {"id": "depmodel", "label": "依赖图模型", "x": 250, "y": 370, "width": 130, "height": 50},
        {"id": "vermodel", "label": "版本与基线模型", "x": 410, "y": 370, "width": 150, "height": 50},
        {"id": "wfmodel", "label": "工作流模型", "x": 590, "y": 370, "width": 130, "height": 50},
        {"id": "policymodel", "label": "规则与策略模型", "x": 740, "y": 370, "width": 150, "height": 50},
        {"id": "pg", "label": "PostgreSQL", "kind": "cylinder", "x": 140, "y": 480, "width": 140, "height": 60},
        {"id": "worker", "label": "异步 Worker", "x": 320, "y": 480, "width": 130, "height": 50},
        {"id": "external", "label": "外部系统连接器", "x": 490, "y": 480, "width": 160, "height": 50}
    ],
    "arrows": [
        {"source": "rest", "target": "assetsvc", "flow": "control"},
        {"source": "mcp", "target": "assetsvc", "flow": "control"},
        {"source": "hooks", "target": "wfsvc", "flow": "control"},
        {"source": "assetsvc", "target": "assetmodel", "flow": "read"},
        {"source": "versionsvc", "target": "vermodel", "flow": "read"},
        {"source": "depsvc", "target": "depmodel", "flow": "read"},
        {"source": "statesvc", "target": "assetmodel", "flow": "read"},
        {"source": "wfsvc", "target": "wfmodel", "flow": "read"},
        {"source": "assetsvc", "target": "pg", "flow": "write"},
        {"source": "versionsvc", "target": "pg", "flow": "write"},
        {"source": "wfsvc", "target": "worker", "flow": "async"},
        {"source": "external", "target": "pg", "flow": "write"}
    ],
    "legend": [
        {"flow": "control", "label": "控制/交互"},
        {"flow": "read", "label": "领域读取"},
        {"flow": "write", "label": "持久化存储"},
        {"flow": "async", "label": "异步任务"}
    ]
})

# ========================================================================
print("Generating 04-03-module-relations...")
generate("04-03-module-relations", "flowchart", {
    "title": "ADAM 模块关系",
    "style": STYLE,
    "style_overrides": FONT_OVERRIDE,
    "viewBox": "0 0 960 520",
    "nodes": [
        {"id": "asset", "label": "资产管理", "x": 80, "y": 160, "width": 140, "height": 55},
        {"id": "version", "label": "版本发布", "x": 280, "y": 160, "width": 140, "height": 55},
        {"id": "dep", "label": "依赖图", "x": 480, "y": 160, "width": 140, "height": 55},
        {"id": "state", "label": "状态传播", "x": 680, "y": 160, "width": 140, "height": 55},
        {"id": "workflow", "label": "工作流自动化", "x": 680, "y": 280, "width": 160, "height": 55},
        {"id": "agent", "label": "Agent 任务", "x": 500, "y": 400, "width": 140, "height": 55},
        {"id": "approval", "label": "人工审批", "x": 700, "y": 400, "width": 140, "height": 55}
    ],
    "arrows": [
        {"source": "asset", "target": "version", "flow": "control"},
        {"source": "version", "target": "dep", "flow": "control"},
        {"source": "dep", "target": "state", "flow": "control"},
        {"source": "state", "target": "workflow", "flow": "control"},
        {"source": "workflow", "target": "agent", "flow": "async"},
        {"source": "workflow", "target": "approval", "flow": "async"},
        {"source": "agent", "target": "version", "flow": "feedback"},
        {"source": "approval", "target": "state", "flow": "feedback"}
    ]
})

# ========================================================================
print("Generating 04-04-runtime-components...")
generate("04-04-runtime-components", "architecture", {
    "title": "运行时组件",
    "style": STYLE,
    "style_overrides": FONT_OVERRIDE,
    "viewBox": "0 0 960 600",
    "nodes": [
        {"id": "user", "label": "研发人员 / 管理界面", "x": 60, "y": 80, "width": 180, "height": 50},
        {"id": "agent", "label": "AI Agent", "x": 60, "y": 180, "width": 120, "height": 50},
        {"id": "external", "label": "Git / Wiki / PM / CI", "x": 60, "y": 280, "width": 180, "height": 50},
        {"id": "rest", "label": "REST API", "x": 340, "y": 80, "width": 130, "height": 50},
        {"id": "mcp", "label": "MCP Server", "x": 340, "y": 180, "width": 130, "height": 50},
        {"id": "adapter", "label": "外部系统适配器", "x": 340, "y": 280, "width": 150, "height": 50},
        {"id": "app", "label": "应用服务", "x": 580, "y": 180, "width": 130, "height": 50},
        {"id": "db", "label": "PostgreSQL", "kind": "cylinder", "x": 780, "y": 80, "width": 140, "height": 60},
        {"id": "event", "label": "WorkflowEvent /\nDirtyQueue", "x": 580, "y": 320, "width": 180, "height": 60},
        {"id": "worker", "label": "异步 Worker", "x": 780, "y": 320, "width": 140, "height": 50},
        {"id": "connector", "label": "外部系统连接器", "x": 780, "y": 440, "width": 150, "height": 50}
    ],
    "arrows": [
        {"source": "user", "target": "rest", "flow": "control"},
        {"source": "agent", "target": "mcp", "flow": "control"},
        {"source": "external", "target": "adapter", "flow": "control"},
        {"source": "rest", "target": "app", "flow": "control"},
        {"source": "mcp", "target": "app", "flow": "control"},
        {"source": "adapter", "target": "app", "flow": "control"},
        {"source": "app", "target": "db", "flow": "write"},
        {"source": "app", "target": "event", "flow": "write"},
        {"source": "event", "target": "worker", "flow": "async"},
        {"source": "worker", "target": "app", "flow": "control"},
        {"source": "worker", "target": "connector", "flow": "control"},
        {"source": "connector", "target": "external", "flow": "data"}
    ]
})

# ========================================================================
print("Generating 05-01-dependency-dag...")
generate("05-01-dependency-dag", "flowchart", {
    "title": "ADAM 资产依赖 DAG",
    "subtitle": "箭头方向：左侧依赖右侧（下游 → 上游）",
    "style": STYLE,
    "style_overrides": FONT_OVERRIDE,
    "viewBox": "0 0 960 520",
    "nodes": [
        {"id": "commit", "label": "code_commit", "x": 80, "y": 280, "width": 150, "height": 50},
        {"id": "workitem", "label": "work_item", "x": 300, "y": 220, "width": 150, "height": 50},
        {"id": "req", "label": "requirement", "x": 540, "y": 160, "width": 160, "height": 50},
        {"id": "design", "label": "design_doc", "x": 300, "y": 340, "width": 150, "height": 50},
        {"id": "testcase", "label": "test_case", "x": 540, "y": 340, "width": 150, "height": 50},
        {"id": "pipeline", "label": "pipeline", "x": 80, "y": 400, "width": 150, "height": 50}
    ],
    "arrows": [
        {"source": "commit", "target": "workitem", "flow": "control"},
        {"source": "workitem", "target": "req", "flow": "control"},
        {"source": "design", "target": "req", "flow": "control"},
        {"source": "testcase", "target": "req", "flow": "control"},
        {"source": "pipeline", "target": "commit", "flow": "control"}
    ]
})

# ========================================================================
print("Generating 05-03-publish-baseline...")
generate("05-03-publish-baseline", "flowchart", {
    "title": "发布依赖快照与当前有效基线",
    "style": STYLE,
    "style_overrides": FONT_OVERRIDE,
    "viewBox": "0 0 960 520",
    "nodes": [
        {"id": "publish", "label": "资产发布", "x": 120, "y": 180, "width": 140, "height": 50},
        {"id": "snapshot", "label": "写入发布依赖快照", "x": 340, "y": 140, "width": 180, "height": 50},
        {"id": "baseline", "label": "更新当前有效依赖基线", "x": 340, "y": 240, "width": 200, "height": 50},
        {"id": "audit", "label": "历史追溯 / 审计", "x": 600, "y": 140, "width": 160, "height": 50},
        {"id": "context", "label": "AI 上下文查询", "x": 600, "y": 220, "width": 160, "height": 50},
        {"id": "dirty", "label": "Dirty 判断", "x": 600, "y": 300, "width": 140, "height": 50},
        {"id": "manual", "label": "手工 Clean", "x": 120, "y": 340, "width": 140, "height": 50},
        {"id": "log", "label": "DirtyResolutionLog", "x": 360, "y": 340, "width": 180, "height": 50}
    ],
    "arrows": [
        {"source": "publish", "target": "snapshot", "flow": "control"},
        {"source": "publish", "target": "baseline", "flow": "control"},
        {"source": "snapshot", "target": "audit", "flow": "data"},
        {"source": "baseline", "target": "context", "flow": "data"},
        {"source": "baseline", "target": "dirty", "flow": "data"},
        {"source": "manual", "target": "baseline", "flow": "control"},
        {"source": "manual", "target": "log", "flow": "write"}
    ]
})

# ========================================================================
print("Generating 06-01-asset-state-machine...")
generate("06-01-asset-state-machine", "state-machine", {
    "title": "资产状态转换",
    "style": STYLE,
    "style_overrides": FONT_OVERRIDE,
    "viewBox": "0 0 960 520",
    "nodes": [
        {"id": "init", "label": "[*]", "kind": "circle", "x": 200, "y": 160, "r": 12, "fill": "#111827"},
        {"id": "clean", "label": "Clean", "x": 320, "y": 160, "width": 140, "height": 50},
        {"id": "dirty", "label": "Dirty", "x": 320, "y": 300, "width": 140, "height": 50},
        {"id": "archived", "label": "Archived", "x": 320, "y": 440, "width": 140, "height": 50}
    ],
    "arrows": [
        {"source": "init", "target": "clean", "flow": "control"},
        {"source": "clean", "target": "dirty", "label": "上游发布 Dirty", "flow": "control"},
        {"source": "dirty", "target": "clean", "label": "重新发布", "flow": "control", "x1": 460, "y1": 325, "x2": 540, "y2": 325},
        {"source": "dirty", "target": "clean", "label": "手工 Clean", "flow": "control", "x1": 320, "y1": 325, "x2": 240, "y2": 325},
        {"source": "clean", "target": "archived", "label": "手动归档", "flow": "control"},
        {"source": "dirty", "target": "archived", "label": "手动归档", "flow": "control"}
    ]
})

# ========================================================================
# Sequence diagrams: use wider viewBox and larger node spacing
# ========================================================================
print("Generating 06-02-dirty-propagation...")
generate("06-02-dirty-propagation", "flowchart", {
    "title": "Dirty 传播规则",
    "style": STYLE,
    "style_overrides": FONT_OVERRIDE,
    "viewBox": "0 0 960 720",
    "nodes": [
        {"id": "upstream", "label": "上游资产发布新版本", "x": 340, "y": 100, "width": 280, "height": 50},
        {"id": "version1", "label": "版本发布服务\n查询直接下游", "x": 340, "y": 200, "width": 280, "height": 65},
        {"id": "graph", "label": "依赖图\n返回依赖边与传播策略", "x": 340, "y": 310, "width": 280, "height": 65},
        {"id": "version2", "label": "版本发布服务\n触发发布事件", "x": 340, "y": 420, "width": 280, "height": 65},
        {"id": "state", "label": "状态传播服务\n过滤 ContextOnly / AuditOnly", "x": 340, "y": 530, "width": 280, "height": 65},
        {"id": "downstream", "label": "下游资产：标记 Dirty\n并写入 Dirty 队列", "x": 340, "y": 640, "width": 280, "height": 65}
    ],
    "arrows": [
        {"source": "upstream", "target": "version1", "flow": "control"},
        {"source": "version1", "target": "graph", "flow": "control"},
        {"source": "graph", "target": "version2", "flow": "data"},
        {"source": "version2", "target": "state", "flow": "control"},
        {"source": "state", "target": "downstream", "flow": "write"}
    ]
})

# ========================================================================
print("Generating 07-01-event-driven-loop...")
generate("07-01-event-driven-loop", "flowchart", {
    "title": "事件驱动闭环",
    "style": STYLE,
    "style_overrides": FONT_OVERRIDE,
    "viewBox": "0 0 960 680",
    "nodes": [
        {"id": "event", "label": "WorkflowEvent\n资产或外部事件", "x": 120, "y": 160, "width": 160, "height": 65},
        {"id": "rule", "label": "PromotionRule\n规则匹配", "x": 360, "y": 160, "width": 160, "height": 65},
        {"id": "action", "label": "WorkflowAction\n流程动作", "x": 600, "y": 160, "width": 160, "height": 65},
        {"id": "executor", "label": "执行方式", "kind": "hexagon", "x": 380, "y": 300, "width": 160, "height": 65},
        {"id": "agent", "label": "AgentTask\nAI Agent 领取执行", "x": 200, "y": 420, "width": 160, "height": 65},
        {"id": "human", "label": "ApprovalGate\n人工审批", "x": 400, "y": 420, "width": 160, "height": 65},
        {"id": "external", "label": "外部系统命令", "x": 600, "y": 420, "width": 160, "height": 55},
        {"id": "result", "label": "结果 / 产出资产", "x": 400, "y": 540, "width": 160, "height": 50},
        {"id": "publish", "label": "发布或更新资产", "x": 400, "y": 620, "width": 160, "height": 50}
    ],
    "arrows": [
        {"source": "event", "target": "rule", "flow": "control"},
        {"source": "rule", "target": "action", "flow": "control"},
        {"source": "action", "target": "executor", "flow": "control"},
        {"source": "executor", "target": "agent", "flow": "async"},
        {"source": "executor", "target": "human", "flow": "async"},
        {"source": "executor", "target": "external", "flow": "async"},
        {"source": "agent", "target": "result", "flow": "data"},
        {"source": "human", "target": "result", "flow": "data"},
        {"source": "external", "target": "result", "flow": "data"},
        {"source": "result", "target": "publish", "flow": "control"}
    ]
})

# ========================================================================
print("Generating 07-02-requirement-to-feature...")
generate("07-02-requirement-to-feature", "sequence", {
    "title": "需求发布到功能工作",
    "style": STYLE,
    "style_overrides": FONT_OVERRIDE,
    "viewBox": "0 0 1400 560",
    "nodes": [
        {"id": "user", "label": "用户/外部系统", "x": 80, "y": 120, "width": 140, "height": 50},
        {"id": "version", "label": "版本发布服务", "x": 340, "y": 120, "width": 140, "height": 50},
        {"id": "event", "label": "WorkflowEvent", "x": 600, "y": 120, "width": 130, "height": 50},
        {"id": "rule", "label": "PromotionRuleEvaluator", "x": 860, "y": 120, "width": 190, "height": 50},
        {"id": "action", "label": "WorkflowActionService", "x": 860, "y": 280, "width": 170, "height": 50},
        {"id": "asset", "label": "资产服务", "x": 600, "y": 420, "width": 130, "height": 50}
    ],
    "arrows": [
        {"source": "user", "target": "version", "label": "发布 requirement", "flow": "control"},
        {"source": "version", "target": "event", "label": "记录 AssetPublished", "flow": "write"},
        {"source": "event", "target": "rule", "label": "触发规则评估", "flow": "control"},
        {"source": "rule", "target": "action", "label": "创建 create_work_item 动作", "flow": "control"},
        {"source": "action", "target": "asset", "label": "创建或关联 work_item(kind=feature)", "flow": "write"},
        {"source": "asset", "target": "action", "label": "返回工作项", "flow": "data"},
        {"source": "action", "target": "event", "label": "记录 WorkItemCreated", "flow": "write"}
    ]
})

# ========================================================================
print("Generating 07-02-dirty-to-review...")
generate("07-02-dirty-to-review", "flowchart", {
    "title": "Dirty 资产到人工审查",
    "style": STYLE,
    "style_overrides": FONT_OVERRIDE,
    "viewBox": "0 0 960 720",
    "nodes": [
        {"id": "state", "label": "状态传播服务", "x": 340, "y": 100, "width": 280, "height": 50},
        {"id": "event1", "label": "WorkflowEvent\n记录 AssetMarkedDirty", "x": 340, "y": 200, "width": 280, "height": 60},
        {"id": "rule", "label": "PromotionRuleEvaluator", "x": 340, "y": 310, "width": 280, "height": 50},
        {"id": "approval", "label": "ApprovalGate", "x": 340, "y": 410, "width": 280, "height": 50},
        {"id": "user", "label": "审查人", "x": 340, "y": 510, "width": 280, "height": 50},
        {"id": "event2", "label": "WorkflowEvent\nHumanApprovalGranted 或 Rejected", "x": 340, "y": 610, "width": 280, "height": 60}
    ],
    "arrows": [
        {"source": "state", "target": "event1", "label": "记录 AssetMarkedDirty", "flow": "write"},
        {"source": "event1", "target": "rule", "label": "匹配 review_dirty_asset 规则", "flow": "control"},
        {"source": "rule", "target": "approval", "label": "创建人工审批/审查门禁", "flow": "control"},
        {"source": "approval", "target": "user", "label": "等待审查", "flow": "control"},
        {"source": "user", "target": "approval", "label": "审查并决定", "flow": "control"},
        {"source": "approval", "target": "event2", "label": "审批结果", "flow": "write"}
    ]
})

# ========================================================================
print("Generating 07-02-agent-task-execution...")
generate("07-02-agent-task-execution", "flowchart", {
    "title": "Agent 任务执行",
    "style": STYLE,
    "style_overrides": FONT_OVERRIDE,
    "viewBox": "0 0 960 880",
    "nodes": [
        {"id": "step1", "label": "工作流服务 → AgentTaskService\n创建 AgentTask", "x": 280, "y": 100, "width": 400, "height": 60},
        {"id": "step2", "label": "AI Agent → AgentTaskService\nlist_pending_agent_tasks", "x": 280, "y": 210, "width": 400, "height": 60},
        {"id": "step3", "label": "AI Agent → AgentTaskService\nclaim_agent_task", "x": 280, "y": 320, "width": 400, "height": 60},
        {"id": "step4", "label": "AgentTaskService → AI Agent\n返回上下文和任务参数", "x": 280, "y": 430, "width": 400, "height": 60},
        {"id": "step5", "label": "AI Agent → AgentTaskService\nsubmit_agent_task_result", "x": 280, "y": 540, "width": 400, "height": 60},
        {"id": "step6", "label": "AgentTaskService → 资产服务\n关联产出资产", "x": 280, "y": 650, "width": 400, "height": 60},
        {"id": "step7", "label": "AgentTaskService → 工作流服务\n触发 AgentTaskSucceeded", "x": 280, "y": 760, "width": 400, "height": 60}
    ],
    "arrows": [
        {"source": "step1", "target": "step2", "flow": "control"},
        {"source": "step2", "target": "step3", "flow": "control"},
        {"source": "step3", "target": "step4", "flow": "control"},
        {"source": "step4", "target": "step5", "flow": "control"},
        {"source": "step5", "target": "step6", "flow": "control"},
        {"source": "step6", "target": "step7", "flow": "control"}
    ]
})

# ========================================================================
print("Generating 07-03-workflow-state-machine...")
generate("07-03-workflow-state-machine", "state-machine", {
    "title": "工作流状态机",
    "style": STYLE,
    "style_overrides": FONT_OVERRIDE,
    "viewBox": "0 0 960 620",
    "nodes": [
        {"id": "init", "label": "[*]", "kind": "circle", "x": 200, "y": 140, "r": 12, "fill": "#111827"},
        {"id": "pending", "label": "Pending", "x": 320, "y": 140, "width": 130, "height": 50},
        {"id": "ready", "label": "Ready", "x": 320, "y": 240, "width": 130, "height": 50},
        {"id": "inprogress", "label": "InProgress", "x": 320, "y": 340, "width": 130, "height": 50},
        {"id": "waitingreview", "label": "WaitingReview", "x": 520, "y": 240, "width": 150, "height": 50},
        {"id": "waitingval", "label": "WaitingValidation", "x": 520, "y": 340, "width": 170, "height": 50},
        {"id": "blocked", "label": "Blocked", "x": 520, "y": 440, "width": 130, "height": 50},
        {"id": "completed", "label": "Completed", "x": 320, "y": 460, "width": 140, "height": 50},
        {"id": "failed", "label": "Failed", "x": 520, "y": 540, "width": 130, "height": 50},
        {"id": "cancelled", "label": "Cancelled", "x": 120, "y": 460, "width": 140, "height": 50}
    ],
    "arrows": [
        {"source": "init", "target": "pending", "flow": "control"},
        {"source": "pending", "target": "ready", "flow": "control"},
        {"source": "ready", "target": "inprogress", "flow": "control"},
        {"source": "inprogress", "target": "waitingreview", "flow": "control"},
        {"source": "inprogress", "target": "waitingval", "flow": "control"},
        {"source": "inprogress", "target": "blocked", "flow": "control"},
        {"source": "inprogress", "target": "completed", "flow": "control"},
        {"source": "inprogress", "target": "failed", "flow": "control"},
        {"source": "waitingreview", "target": "ready", "flow": "control"},
        {"source": "waitingreview", "target": "completed", "flow": "control"},
        {"source": "waitingval", "target": "ready", "flow": "control"},
        {"source": "waitingval", "target": "completed", "flow": "control"},
        {"source": "blocked", "target": "ready", "flow": "control"},
        {"source": "blocked", "target": "failed", "flow": "control"},
        {"source": "ready", "target": "cancelled", "flow": "control"},
        {"source": "inprogress", "target": "cancelled", "flow": "control"}
    ]
})

# ========================================================================
print("Generating 07-04-failure-compensation-dlq...")
generate("07-04-failure-compensation-dlq", "flowchart", {
    "title": "失败、补偿与死信",
    "style": STYLE,
    "style_overrides": FONT_OVERRIDE,
    "viewBox": "0 0 960 640",
    "nodes": [
        {"id": "failure", "label": "动作失败", "x": 200, "y": 160, "width": 140, "height": 50},
        {"id": "retry", "label": "还有重试预算？", "kind": "hexagon", "x": 420, "y": 160, "width": 160, "height": 65},
        {"id": "pending", "label": "回到 Pending\n等待 next_retry_at", "x": 640, "y": 120, "width": 180, "height": 65},
        {"id": "sideeffect", "label": "已有外部副作用？", "kind": "hexagon", "x": 420, "y": 300, "width": 160, "height": 65},
        {"id": "failed", "label": "动作 Failed", "x": 640, "y": 300, "width": 140, "height": 50},
        {"id": "compensate", "label": "有补偿策略？", "kind": "hexagon", "x": 420, "y": 440, "width": 160, "height": 65},
        {"id": "compensation", "label": "创建补偿动作", "x": 640, "y": 440, "width": 160, "height": 50},
        {"id": "manual", "label": "Blocked:\nWaitingManualIntervention", "x": 200, "y": 440, "width": 180, "height": 65},
        {"id": "dlq", "label": "workflow_dead_letters", "x": 200, "y": 560, "width": 180, "height": 50}
    ],
    "arrows": [
        {"source": "failure", "target": "retry", "flow": "control"},
        {"source": "retry", "target": "pending", "label": "是", "flow": "control"},
        {"source": "retry", "target": "sideeffect", "label": "否", "flow": "control"},
        {"source": "sideeffect", "target": "failed", "label": "否", "flow": "control"},
        {"source": "sideeffect", "target": "compensate", "label": "是", "flow": "control"},
        {"source": "compensate", "target": "compensation", "label": "有", "flow": "control"},
        {"source": "compensate", "target": "manual", "label": "无", "flow": "control"},
        {"source": "manual", "target": "dlq", "flow": "control"}
    ]
})

# ========================================================================
print("Generating 09-01-core-data-model...")
generate("09-01-core-data-model", "er-diagram", {
    "title": "核心数据对象",
    "style": STYLE,
    "style_overrides": {**FONT_OVERRIDE, "node_title_size": 13, "type_label_size": 10},
    "viewBox": "0 0 1200 640",
    "nodes": [
        {"id": "assettype", "label": "ASSET_TYPE", "x": 60, "y": 80, "width": 120, "height": 45, "type_label": "ENTITY"},
        {"id": "assetinst", "label": "ASSET_INSTANCE", "x": 280, "y": 80, "width": 140, "height": 45, "type_label": "ENTITY"},
        {"id": "assetver", "label": "ASSET_VERSION", "x": 540, "y": 80, "width": 140, "height": 45, "type_label": "ENTITY"},
        {"id": "virtinst", "label": "VIRTUAL_INSTANCE", "x": 780, "y": 80, "width": 150, "height": 45, "type_label": "ENTITY"},
        {"id": "assetdep", "label": "ASSET_DEPENDENCY", "x": 280, "y": 200, "width": 160, "height": 45, "type_label": "ENTITY"},
        {"id": "dirtyq", "label": "DIRTY_QUEUE", "x": 540, "y": 200, "width": 130, "height": 45, "type_label": "ENTITY"},
        {"id": "dirtylog", "label": "DIRTY_RESOLUTION_LOG", "x": 780, "y": 200, "width": 180, "height": 45, "type_label": "ENTITY"},
        {"id": "wfevent", "label": "WORKFLOW_EVENT", "x": 60, "y": 320, "width": 140, "height": 45, "type_label": "ENTITY"},
        {"id": "wfinstance", "label": "WORKFLOW_INSTANCE", "x": 280, "y": 320, "width": 160, "height": 45, "type_label": "ENTITY"},
        {"id": "wfaction", "label": "WORKFLOW_ACTION", "x": 540, "y": 320, "width": 150, "height": 45, "type_label": "ENTITY"},
        {"id": "deadletter", "label": "WORKFLOW_DEAD_LETTER", "x": 280, "y": 440, "width": 180, "height": 45, "type_label": "ENTITY"},
        {"id": "agenttask", "label": "AGENT_TASK", "x": 540, "y": 440, "width": 130, "height": 45, "type_label": "ENTITY"},
        {"id": "approval", "label": "APPROVAL_GATE", "x": 780, "y": 440, "width": 150, "height": 45, "type_label": "ENTITY"}
    ],
    "arrows": [
        {"source": "assettype", "target": "assetinst", "label": "classifies 1:N", "flow": "control", "label_dy": -20},
        {"source": "assetinst", "target": "assetver", "label": "publishes 1:N", "flow": "control", "label_dy": -20},
        {"source": "assetinst", "target": "assetdep", "label": "source/target 1:N", "flow": "control"},
        {"source": "assetinst", "target": "dirtyq", "label": "affected 1:N", "flow": "control"},
        {"source": "assetver", "target": "dirtylog", "label": "reviewed 1:N", "flow": "control"},
        {"source": "assetver", "target": "virtinst", "label": "anchors 1:N", "flow": "control", "label_dy": -20},
        {"source": "wfevent", "target": "wfaction", "label": "causes 1:N", "flow": "control", "route_points": [[200, 280], [540, 280]], "label_dy": -20},
        {"source": "wfinstance", "target": "wfaction", "label": "contains 1:N", "flow": "control", "label_dy": -20},
        {"source": "wfaction", "target": "agenttask", "label": "executes 1:N", "flow": "control"},
        {"source": "wfaction", "target": "approval", "label": "requires 1:N", "flow": "control"},
        {"source": "wfaction", "target": "deadletter", "label": "fails_into 1:N", "flow": "control"}
    ]
})

# ========================================================================
print("Generating 10-03-external-integration...")
generate("10-03-external-integration", "architecture", {
    "title": "外部系统接入",
    "style": STYLE,
    "style_overrides": FONT_OVERRIDE,
    "viewBox": "0 0 960 560",
    "nodes": [
        {"id": "git", "label": "Git Hook / Webhook", "x": 80, "y": 160, "width": 160, "height": 50},
        {"id": "ci", "label": "CI/CD Pipeline", "x": 80, "y": 280, "width": 160, "height": 50},
        {"id": "wiki", "label": "Wiki / 文档系统", "x": 80, "y": 400, "width": 160, "height": 50},
        {"id": "pm", "label": "项目管理系统", "x": 80, "y": 480, "width": 160, "height": 50},
        {"id": "publish", "label": "资产发布接口", "x": 340, "y": 220, "width": 160, "height": 50},
        {"id": "pipeline", "label": "流水线执行记录", "x": 340, "y": 380, "width": 170, "height": 50},
        {"id": "adam", "label": "ADAM", "kind": "double_rect", "x": 620, "y": 280, "width": 160, "height": 80, "accent_fill": "#7c3aed"}
    ],
    "arrows": [
        {"source": "git", "target": "publish", "flow": "control"},
        {"source": "wiki", "target": "publish", "flow": "control"},
        {"source": "pm", "target": "publish", "flow": "control"},
        {"source": "publish", "target": "adam", "flow": "write"},
        {"source": "ci", "target": "pipeline", "flow": "control"},
        {"source": "pipeline", "target": "adam", "flow": "write"}
    ]
})

# ========================================================================
print("Generating 10-02-context-construction...")
generate("10-02-context-construction", "flowchart", {
    "title": "上下文构造流程",
    "style": STYLE,
    "style_overrides": FONT_OVERRIDE,
    "viewBox": "0 0 960 820",
    "nodes": [
        {"id": "entry", "label": "入口对象\nAsset / WorkflowAction / AgentTask", "x": 300, "y": 80, "width": 360, "height": 60},
        {"id": "scope", "label": "解析组织、项目和权限边界", "x": 300, "y": 180, "width": 360, "height": 50},
        {"id": "strategy", "label": "选择上下文策略", "x": 300, "y": 270, "width": 360, "height": 50},
        {"id": "graph", "label": "遍历资产依赖图", "x": 300, "y": 360, "width": 360, "height": 50},
        {"id": "version", "label": "解析版本快照与当前基线", "x": 300, "y": 450, "width": 360, "height": 50},
        {"id": "state", "label": "合并 Dirty / Workflow 状态", "x": 300, "y": 540, "width": 360, "height": 50},
        {"id": "filter", "label": "权限、关系类型和预算过滤", "x": 300, "y": 630, "width": 360, "height": 50},
        {"id": "context", "label": "结构化 Agent Context", "x": 300, "y": 720, "width": 360, "height": 50}
    ],
    "arrows": [
        {"source": "entry", "target": "scope", "flow": "control"},
        {"source": "scope", "target": "strategy", "flow": "control"},
        {"source": "strategy", "target": "graph", "flow": "control"},
        {"source": "graph", "target": "version", "flow": "control"},
        {"source": "version", "target": "state", "flow": "control"},
        {"source": "state", "target": "filter", "flow": "control"},
        {"source": "filter", "target": "context", "flow": "control"}
    ]
})

# ========================================================================
print("Generating 14-01-requirement-driven-dev...")
generate("14-01-requirement-driven-dev", "flowchart", {
    "title": "需求驱动功能开发",
    "style": STYLE,
    "style_overrides": FONT_OVERRIDE,
    "viewBox": "0 0 960 720",
    "nodes": [
        {"id": "req", "label": "需求发布", "x": 80, "y": 160, "width": 140, "height": 50},
        {"id": "event", "label": "AssetPublished\n(requirement)", "x": 280, "y": 160, "width": 200, "height": 60},
        {"id": "rule", "label": "PromotionRule", "x": 540, "y": 160, "width": 150, "height": 50},
        {"id": "work", "label": "创建 work_item\n(kind=feature)", "x": 280, "y": 280, "width": 220, "height": 60},
        {"id": "context", "label": "创建虚拟上下文", "x": 540, "y": 280, "width": 160, "height": 50},
        {"id": "agent", "label": "Agent 生成代码变更建议", "x": 280, "y": 400, "width": 220, "height": 50},
        {"id": "commit", "label": "发布 code_commit", "x": 540, "y": 400, "width": 170, "height": 50},
        {"id": "pipeline", "label": "运行 pipeline", "x": 280, "y": 520, "width": 160, "height": 50},
        {"id": "done", "label": "验证通过？", "kind": "hexagon", "x": 480, "y": 520, "width": 150, "height": 55},
        {"id": "complete", "label": "工作流完成", "x": 480, "y": 640, "width": 150, "height": 50},
        {"id": "bugfix", "label": "创建 bugfix 或人工分诊", "x": 700, "y": 640, "width": 200, "height": 50}
    ],
    "arrows": [
        {"source": "req", "target": "event", "flow": "control"},
        {"source": "event", "target": "rule", "flow": "control"},
        {"source": "rule", "target": "work", "flow": "control"},
        {"source": "work", "target": "context", "flow": "control"},
        {"source": "context", "target": "agent", "flow": "control"},
        {"source": "agent", "target": "commit", "flow": "control"},
        {"source": "commit", "target": "pipeline", "flow": "control"},
        {"source": "pipeline", "target": "done", "flow": "control"},
        {"source": "done", "target": "complete", "label": "是", "flow": "control"},
        {"source": "done", "target": "bugfix", "label": "否", "flow": "control"}
    ]
})

# ========================================================================
print("Generating 14-02-requirement-change-impact...")
generate("14-02-requirement-change-impact", "flowchart", {
    "title": "需求变更影响测试用例",
    "style": STYLE,
    "style_overrides": FONT_OVERRIDE,
    "viewBox": "0 0 960 600",
    "nodes": [
        {"id": "reqv2", "label": "requirement 发布 v2", "x": 80, "y": 160, "width": 180, "height": 50},
        {"id": "graph", "label": "查找直接下游", "x": 340, "y": 160, "width": 150, "height": 50},
        {"id": "tc", "label": "test_case\nverifies requirement", "x": 560, "y": 160, "width": 220, "height": 60},
        {"id": "dirty", "label": "test_case Dirty", "x": 340, "y": 280, "width": 150, "height": 50},
        {"id": "review", "label": "创建 review_dirty_asset", "x": 560, "y": 280, "width": 210, "height": 50},
        {"id": "decision", "label": "审查结果", "kind": "hexagon", "x": 400, "y": 400, "width": 150, "height": 55},
        {"id": "clean", "label": "手工 Clean", "x": 200, "y": 520, "width": 140, "height": 50},
        {"id": "republish", "label": "更新并重新发布 test_case", "x": 560, "y": 520, "width": 220, "height": 50}
    ],
    "arrows": [
        {"source": "reqv2", "target": "graph", "flow": "control"},
        {"source": "graph", "target": "tc", "flow": "control"},
        {"source": "tc", "target": "dirty", "flow": "control"},
        {"source": "dirty", "target": "review", "flow": "control"},
        {"source": "review", "target": "decision", "flow": "control"},
        {"source": "decision", "target": "clean", "label": "无影响", "flow": "control"},
        {"source": "decision", "target": "republish", "label": "需更新", "flow": "control"}
    ]
})

# ========================================================================
print("Generating 14-03-pipeline-failure-fix...")
generate("14-03-pipeline-failure-fix", "flowchart", {
    "title": "流水线失败触发修复流程",
    "style": STYLE,
    "style_overrides": FONT_OVERRIDE,
    "viewBox": "0 0 960 600",
    "nodes": [
        {"id": "fail", "label": "PipelineRunFailed", "x": 80, "y": 160, "width": 160, "height": 50},
        {"id": "rule", "label": "PromotionRule", "x": 300, "y": 160, "width": 150, "height": 50},
        {"id": "gate", "label": "ApprovalGate:\n是否创建 bugfix", "x": 500, "y": 140, "width": 170, "height": 65},
        {"id": "bugfix", "label": "work_item(kind=bugfix)", "x": 740, "y": 160, "width": 190, "height": 50},
        {"id": "context", "label": "关联 requirement /\ntest_case / code context", "x": 300, "y": 300, "width": 280, "height": 60},
        {"id": "agent", "label": "Agent 生成修复建议", "x": 640, "y": 300, "width": 180, "height": 50},
        {"id": "commit", "label": "发布 code_commit", "x": 300, "y": 420, "width": 170, "height": 50},
        {"id": "retry", "label": "重新运行 pipeline", "x": 540, "y": 420, "width": 180, "height": 50}
    ],
    "arrows": [
        {"source": "fail", "target": "rule", "flow": "control"},
        {"source": "rule", "target": "gate", "flow": "control"},
        {"source": "gate", "target": "bugfix", "label": "批准", "flow": "control"},
        {"source": "bugfix", "target": "context", "flow": "control"},
        {"source": "context", "target": "agent", "flow": "control"},
        {"source": "agent", "target": "commit", "flow": "control"},
        {"source": "commit", "target": "retry", "flow": "control"}
    ]
})

print("\n=== Batch generation complete ===")
