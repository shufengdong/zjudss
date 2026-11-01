# Workflow Results Implementation Plan

**Goal:** 成功研究工作流后紧跟可查看的运行结果卡，恢复会话后仍可读取。
**Architecture:** 前端按工作流 runId 读取现有存活数据集接口，复用 ResultCard；不新增公共消息或路由。结果只在成功后加载，提供错误重试，组件卸载后丢弃迟到响应。
**Tech Stack:** React / TypeScript / Vitest。
**Spec:** 用户已确认的成功工作流后展示全部结果名称、行列数与查看按钮方案。

- [x] 添加行为测试，验证成功后逐项查看、失败重试及非成功不加载。
- [x] 实现成功工作流后的结果卡，复用已有 API 与 ResultCard。
- [x] 同步前端设计和功能目录，运行相关测试及前端构建。
