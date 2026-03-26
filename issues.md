# Issues & Future Improvements

## Performance

### P3: UIA 轮询 100ms 持续消耗 CPU
- **现状**: `monitor.rs` 每 100ms 调用 UIA `GetFocusedElement` + `GetSelection` 检测选中文本，即使用户没有任何操作也在持续轮询
- **影响**: 后台持续占用 CPU，对笔记本电池不友好
- **可选方案**:
  1. **自适应频率**: 空闲时 500ms 慢轮询，检测到输入框焦点时 100ms 快轮询，popup 可见时暂停（已部分实现）
  2. **UIA 事件订阅**: `AddFocusChangedEventHandler` + `TextSelectionChanged` 事件驱动，但跨进程场景（Teams/VS Code 等 Electron 应用）可能不触发
  3. **混合方案**: UIA 事件 + 1-2s 慢速兜底轮询
- **决定**: 暂不修改，保持 100ms 轮询。功能稳定性优先

## Reliability

### P2: `preview_visible` 标志可能卡住
- **现状**: 已加 watchdog（每个 poll 周期检查 popup 是否实际可见），但本质上是用轮询修轮询
- **理想方案**: 改为事件驱动 — 当用户选中文本触发事件时，检查并修正 `preview_visible` 不一致状态
- **相关 commit**: `6bb6334`

### P3: `unsafe impl Send + Sync for UiaEngine`
- **现状**: UIA COM 对象本身不是线程安全的，当前用 `unsafe impl Sync` 绕过
- **风险**: 如果 UIA 从多个线程调用可能 crash
- **缓解**: 目前只在 monitor 线程使用，实际安全
- **理想方案**: 去掉 `Sync` impl，确保只在创建线程使用

## Security

### P1: GitHub Token 明文存储
- **现状**: Token 存在 `auth.json` + `settings.json`（明文 JSON 文件）
- **风险**: 任何能读用户目录的进程都能拿到 token
- **理想方案**: 使用 Windows Credential Manager 存储
- **来源**: Code Review HI-1

### P2: `dangerouslySetInnerHTML` + LLM 输出 = XSS 风险
- **现状**: LLM 返回的 Markdown 经 `marked` 渲染后直接插入 DOM
- **风险**: 如果 LLM 被 prompt injection 返回 `<script>` 标签，可在 WebView 中执行
- **理想方案**: 加 DOMPurify 清理 HTML
- **来源**: Code Review HI-5

### P3: OAuth device code 无超时
- **现状**: `poll_github_login` 会一直轮询直到成功或失败，无用户端超时
- **理想方案**: 加 10 分钟超时
- **来源**: Code Review HI-1

## Distribution

### P3: Windows SmartScreen 警告 "Unknown Publisher"
- **现状**: 下载 exe 安装包时 Edge 显示警告，运行时 SmartScreen 弹窗 "Windows protected your PC"
- **原因**: 没有代码签名证书（code signing certificate），Publisher 显示为 Unknown
- **方案**:
  - 普通代码签名证书（~$200-400/年）— SmartScreen 警告在用户量积累后逐渐消失
  - EV 代码签名证书（~$400+/年）— 立即消除 SmartScreen 警告
  - Tauri 支持在 `tauri.conf.json` 中配置 Windows 代码签名
- **决定**: 暂不处理，自用阶段直接 "Run anyway"。分发给其他人时再考虑购买证书

### P2: Private Repo 导致 Auto-Updater 404
- **现状**: 已修复 — repo 改为 public
- **原因**: Private repo 的 release assets 无法匿名访问，updater 请求 `latest.json` 返回 404
- **相关 commit**: repo visibility → public

## UX

### P3: Replace 操作无法撤销
- **现状**: `Ctrl+V` 粘贴替换后，用户无法 Ctrl+Z 恢复原文
- **原因**: 剪贴板被覆写，Ctrl+V 在目标应用中可能不支持 undo
- **理想方案**: 在 replace 前保存原文到内部历史，提供 undo 按钮
- **来源**: Code Review HI-3

### P3: `SetForegroundWindow` 可能被 Windows 阻止
- **现状**: Replace 需要把焦点切回源应用，但 Windows 对前台窗口切换有限制
- **缓解**: 已调用 `AllowSetForegroundWindow(ASFW_ANY)`
- **来源**: Code Review HI-4
