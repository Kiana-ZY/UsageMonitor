# web-dashboard 验收报告

> 验收日期：2026-05-08 | 证据：http://127.0.0.1:4317 正常服务

## 验收
- [x] serve 命令启动 axum HTTP 服务
- [x] / 返回仪表盘 HTML
- [x] /api/summary 返回统计（cache_hit_rate: 69.8%）
- [x] /api/models 返回模型列表
- [x] /api/sessions 返回 session 列表
- [x] /api/daily 返回每日趋势
- [x] /api/scan 触发扫描 + 持久化
- [x] 前端 Chart.js 柱状图 + 数据表格
