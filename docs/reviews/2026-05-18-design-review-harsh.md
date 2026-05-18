# 版本线设计文档审查报告 (严苛版)

**审查日期**: 2026-05-18  
**审查对象**: `docs/plans/2026-05-18-version-line-design.md`  
**审查标准**: 架构一致性、实现完整性、性能考量、边界处理  
**总体评分**: **4.8/10 - 不建议按此设计实施**

---

## 🚨 致命缺陷 (必须修复)

### 1. ADR-015 与 ADR-016 逻辑矛盾

**问题描述**:
- ADR-015 定义外部引用格式为 `AssetId@version_line`，暗示不同 version line 中的"相同"资产是不同实体
- ADR-016 提供 `CrossVersionLineService` 进行跨版本线比较和同步
- **矛盾**: 如果 REQ-1@v1.x 和 REQ-1@v2.x 是不同的 AssetId，同步意味着什么？复制？映射？

**根本原因**: Fork 模型的遗留思维与 Asset Version History 架构混淆

**影响**: 架构基础不稳固，实现时必然出现逻辑断裂

**建议**: 
- 完全采用 Asset Version History: 删除 `@version_line` 后缀，AssetId 全局唯一
- 或完全采用 Fork: 保留 AssetId@version_line，删除 CrossVersionLineService

---

### 2. 缺少数据一致性保证

**问题描述**:
```rust
pub struct AssetDependency {
    pub version_constraint: String, // ">=1.0.0, <2.0.0"
}
```

**缺失机制**:
1. ❌ 约束字符串格式验证策略
2. ❌ 破坏性变更处理策略 (REQ-1 从 1.x → 2.0.0)
3. ❌ 依赖约束升级/降级策略
4. ❌ 约束与版本不匹配时的降级路径

**影响**: 运行时错误、数据不一致、用户困惑

---

### 3. Dirty State 内存爆炸风险

**问题代码**:
```rust
pub struct VersionDirtyState {
    pub upstream_triggers: Vec<DirtyTrigger>, // ⚠️ 无限增长
}
```

**风险场景**:
- REQ-1 持续发布 patch: 1.0.1, 1.0.2, 1.0.3...
- CODE-1 依赖 "^1.0.0"
- CODE-1 的 upstream_triggers 累积所有版本
- **结果**: 内存占用随时间线性增长，无清理策略

**建议**: 
```rust
pub struct VersionDirtyState {
    pub latest_trigger: DirtyTrigger,       // 只保留最新
    pub trigger_count: usize,               // 计数
    pub history_truncated: bool,            // 标记已截断
}
```

---

## ⚠️ 严重问题 (必须修复)

### 4. 版本约束解析性能未评估

**问题代码**:
```rust
let constraint = VersionConstraint::parse(&dep.version_constraint)?;
```

**性能隐患**:
- SemVer 解析是 CPU 密集型操作
- 10万依赖 = 10万次解析
- 无缓存机制
- 查询时重复解析

**建议**:
```rust
// 1. 预编译约束
pub struct CompiledConstraint {
    original: String,
    compiled: semver::VersionReq,  // 编译一次，复用多次
}

// 2. 查询缓存
impl AssetRepository {
    pub async fn find_by_constraint_cached(
        &self,
        constraint_str: &str,
    ) -> Result<Vec<Asset>, Error> {
        // 使用编译后的约束，避免重复解析
    }
}
```

---

### 5. 缺少依赖循环检测

**问题描述**:
```
A depends on B ^1.0.0
B depends on C ^1.0.0  
C depends on A ^1.0.0  ← 循环！
```

**现状**: 文档未提及如何处理

**后果**: 
- 无限递归在 Dirty 传播时
- 栈溢出或服务挂起

**建议**:
```rust
impl DependencyService {
    /// 检测循环依赖
    pub fn detect_cycle(
        &self,
        from: AssetId,
    ) -> Result<Option<Vec<AssetId>>, DependencyError> {
        let mut visited: HashSet<AssetId> = HashSet::new();
        let mut path: Vec<AssetId> = Vec::new();
        
        fn dfs(
            current: AssetId,
            visited: &mut HashSet<AssetId>,
            path: &mut Vec<AssetId>,
        ) -> Option<Vec<AssetId>> {
            if path.contains(&current) {
                let cycle_start = path.iter().position(|&x| x == current).unwrap();
                return Some(path[cycle_start..].to_vec());
            }
            
            if visited.contains(&current) {
                return None;
            }
            
            visited.insert(current);
            path.push(current);
            
            // 递归检查依赖
            for dep in get_dependencies(current) {
                if let Some(cycle) = dfs(dep, visited, path) {
                    return Some(cycle);
                }
            }
            
            path.pop();
            None
        }
        
        Ok(dfs(from, &mut visited, &mut path))
    }
}
```

---

### 6. Version Line 查询性能陷阱

**问题 SQL**:
```sql
WHERE v.version = (
    SELECT MAX(version) 
    FROM asset_versions 
    WHERE asset_id = a.id 
    AND version BETWEEN $1 AND $2
)
```

**性能问题**:
1. 子查询导致 N+1 问题
2. 无有效索引支持此查询模式
3. 大数据集时全表扫描

**建议优化**:
```sql
-- 1. 物化版本线成员表
CREATE TABLE version_line_membership (
    version_line_id UUID,
    asset_id UUID,
    current_version_in_line VARCHAR,  -- 该版本线内的当前版本
    PRIMARY KEY (version_line_id, asset_id)
);

-- 2. 或创建覆盖索引
CREATE INDEX idx_asset_versions_line_lookup 
ON asset_versions(asset_id, version, state)
INCLUDE (content_ref);
```

---

## 🔶 中等问题

### 7. 并发发布竞争条件

**问题场景**:
```
T1: REQ-1 发布 1.2.0，开始传播 Dirty
T2: CODE-1 同时发布 1.3.0（已依赖 REQ-1 1.2.0）
结果: 竞态条件，Dirty 状态不确定
```

**缺失**: 并发控制策略

**建议**:
```rust
pub struct AssetVersion {
    pub version: SemVer,
    pub state: AssetVersionState,
    pub lock_version: i64,  // 乐观锁
}

impl AssetLifecycleService {
    pub async fn publish_version(
        &self,
        asset_id: AssetId,
        new_version: SemVer,
        expected_version: i64,  // 乐观锁检查
    ) -> Result<(), Error> {
        // UPDATE ... WHERE lock_version = $expected_version
        // 如果失败，返回 ConflictError
    }
}
```

---

### 8. OrgAssetForkStrategy 在 Asset Version History 中失效

**问题**:
- ADR-014 设计基于 Fork 模型
- 在 Asset Version History 中，Organization-level 资产不需要特殊 Fork 处理

**影响**: 该 ADR 应该被移除或重写

---

### 9. Git 集成复杂性被低估

**未考虑场景**:
1. Git branch 随意创建/删除 vs Version Line 持久化
2. Git rebase 改变 commit hash → Asset external_ref 失效
3. 多人协作：不同人分支映射到同一 Version Line

**建议**: 
- 添加 `external_ref` 失效处理策略
- 定义 Git 工作流规范
- 提供 Git hook 验证工具

---

## 📋 功能遗漏清单

| 功能 | 重要性 | 说明 |
|------|--------|------|
| 资产重命名 | 高 | 未定义重命名流程和影响 |
| 版本撤回 (unpublish) | 高 | 已发布版本能否撤回？影响？ |
| 依赖约束降级 | 中 | ^2.0.0 → ^1.0.0 的逆向操作 |
| 版本线删除 | 高 | 删除策略（软删除？硬删除？） |
| 批量状态更新 | 中 | 如何批量标记 Clean |
| 版本别名 | 低 | "latest", "stable" 指向 |
| 审计日志 | 高 | 谁改了约束？何时？ |

---

## 📊 评分细则

| 维度 | 权重 | 得分 | 说明 |
|------|------|------|------|
| 架构一致性 | 25% | 6/10 | ADR 间存在逻辑矛盾 |
| 实现完整性 | 25% | 5/10 | 缺少关键机制 |
| 性能考量 | 20% | 4/10 | 明显性能陷阱 |
| 边界处理 | 20% | 4/10 | 大量边界未定义 |
| 运维友好性 | 10% | 5/10 | 复杂查询增加运维难度 |
| **加权总分** | 100% | **4.8/10** | **不建议实施** |

---

## 🎯 必须修复的 5 个 blocker

### Blocker 1: 明确架构选择
**行动**: 完全采用 Asset Version History 或完全采用 Fork，删除混用内容
**优先级**: P0
**预计工作量**: 2-3 天重新设计

### Blocker 2: 添加约束验证机制
**行动**: 
- 定义 `VersionConstraint` 类型
- 实现约束解析和验证
- 定义破坏性变更处理策略
**优先级**: P0
**预计工作量**: 1-2 天

### Blocker 3: 解决循环依赖检测
**行动**: 实现 DAG 检测算法，在创建依赖时验证
**优先级**: P0
**预计工作量**: 1 天

### Blocker 4: 优化查询性能
**行动**: 
- 设计物化视图或查询缓存
- 重新定义索引策略
**优先级**: P0
**预计工作量**: 2-3 天

### Blocker 5: 定义并发模型
**行动**: 
- 选择乐观锁或悲观锁
- 实现版本控制机制
**优先级**: P0
**预计工作量**: 1-2 天

---

## 📝 审查结论

### 总体评价

**不推荐按此设计实施**

虽然文档在 ADR 结构和代码示例上表现出色，但存在根本性的架构矛盾和关键实现缺陷。这些问题如果在开发后期发现，将导致大规模重构。

### 建议路径

**选项 A: 修复后实施 (推荐)**
1. 修复 5 个 blocker (约 1-2 周)
2. 重新审查
3. 进入开发

**选项 B: 重新设计**
1. 选择单一架构方向
2. 从头设计核心模型
3. 预计增加 2-3 周设计时间

**选项 C: 简化范围 ( pragmatic )**
1. 放弃 Asset Version History，回到简单 Fork 模型
2. 放弃复杂约束，使用精确版本依赖
3. 快速实施，后续迭代

### 关键决策点

**决策 1**: Asset Version History vs Fork？
- Asset Version History 更优雅但复杂
- Fork 更简单但数据冗余

**决策 2**: 约束系统范围？
- 支持完整 SemVer 范围？
- 仅支持精确版本和 latest？
- 支持通配符 (e.g., "1.x")？

**决策 3**: 性能优先还是功能优先？
- 物化视图 = 功能完整但复杂
- 简单查询 = 性能好但功能受限

---

**审查人**: Claude Code  
**审查完成时间**: 2026-05-18  
**下次审查建议**: 修复 5 个 blocker 后重新审查

---

## 附录: ADR 状态清单

| ADR | 状态 | 说明 |
|-----|------|------|
| ADR-005 | ⚠️ 需调整 | Fork 验证模式，若采用 Asset History 需重写 |
| ADR-012 | ✅ 可用 | 版本继承模式 |
| ADR-013 | ✅ 可用 | Dirty State 行为 |
| ADR-014 | 🔴 需删除 | 基于 Fork 模型，不适用 Asset History |
| ADR-015 | 🔴 需重写 | 逻辑矛盾，需明确 AssetId 策略 |
| ADR-016 | 🔴 需重写 | 基于 Fork 思维，与 Asset History 矛盾 |
| ADR-017 | ✅ 可用 | 迁移策略 |
| ADR-018 | ✅ 可用 | 批量迁移 |
| ADR-019 | ⚠️ 需调整 | 索引设计需适配 Asset History 查询模式 |
| ADR-020 | 🔴 需删除 | Fork 专用，Asset History 不需要 |
