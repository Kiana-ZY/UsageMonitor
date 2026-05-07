# storage-layer 验收报告

> 验收日期：2026-05-08 | 证据：17,826 条持久化成功

## 1-3. 核对
- [x] messages / daily_rollups / model_pricing 三表创建
- [x] insert_messages 批量写入 + UNIQUE 去重
- [x] upsert_daily_rollups 日聚合
- [x] query_daily / query_models / query_sessions 查询 API
- [x] CLI scan 挂接存储

## 5. 架构归并 ✓ | 6. req 无变更 | 7. roadmap items → done | 8-9. 无遗留
