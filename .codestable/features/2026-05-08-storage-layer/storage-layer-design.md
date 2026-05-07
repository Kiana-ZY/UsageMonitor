---
doc_type: feature-design
feature: 2026-05-08-storage-layer
requirement: usage-monitor
roadmap: usage-monitor
roadmap_item: storage-layer
status: approved
summary: SQLite 存储层——messages/daily_rollups/model_pricing 三表 + 批量写入 + 多维度查询 API
tags: [storage, sqlite]
---

# storage-layer

## 0. 术语：按 roadmap 4.3 接口契约

## 1. 决策

- 新 crate `usage-monitor-storage`，依赖 core-engine + rusqlite
- 三张表：messages（请求明细）/ daily_rollups（日聚合）/ model_pricing（定价缓存）
- 批量插入 + 日聚合 upsert
- 查询 API：summary / daily / models / sessions / requests

**不做**：不包含扫描逻辑，只做存/取

## 2. 名词与编排

**现状**：无持久化。

**变化**：`Storage` struct + 三表 schema + 写入/查询方法。

### 挂载点
- `crates/usage-monitor-storage/` crate
- `Cargo.toml` workspace
- CLI scan 挂接存储写入

### 推进
1. Cargo 搭建 + schema 建表
2. 写入 API（insert_messages / upsert_daily）
3. 查询 API
4. 挂接 CLI scan
5. 校验收尾

## 3. 验收

- S1: scan 后数据持久化到 messages 表
- S2: 重复 scan 不重复插入
- S3: 查询 API 返回正确统计

## 4. 架构

ARCHITECTURE.md storage → 已实现
