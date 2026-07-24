# Popup 手动 Resize 设计

## 目标

允许用户像普通窗口一样从 8 个方向（4 边 + 4 角）拖拽调整 popup（expanded/loading/streaming/error 状态）的尺寸。尺寸被记住并跨重启保留，内容超出时内部滚动。

## 行为决策

- **8 向 resize**：上、下、左、右 4 条边 + 4 个角。拖某条边时对边固定（拖上边则下边固定，窗口位置随之上移/下移；拖右下角则左上角固定）。
- **手动尺寸锁定**：去掉「按内容自动测量高度」逻辑，expanded 尺寸完全由记忆值决定。
- **跨 popup / 跨重启记忆**：尺寸（仅宽高，不含位置）存入 `settings.json`，所有 popup 共用同一记忆尺寸。位置仍由选区/光标决定，不记忆。
- **首次默认**：400×300。
- **最小尺寸**：300×200。
- **屏幕边界**：目标 rect（位置+尺寸）clamp 到当前 monitor 可视区内。拖到边缘即拖不动；存储也存 clamp 后的尺寸。
- **内容溢出**：内容区已是 `overflow-auto`，超出自动滚动，无额外改动。

## 实现

### 后端 `src-tauri/src/overlay/mod.rs`

**删除：**
- `resize_popup_to_content()` 函数
- `estimate_height()` 函数
- 常量：`EXPANDED_MIN_HEIGHT`、`EXPANDED_MAX_HEIGHT`、`EXPANDED_STREAMING_HEIGHT`、`BUTTONS_HEIGHT`、`TEXT_PADDING`、`LINE_HEIGHT_PX`、`CHARS_PER_LINE`
- `POPUP_BOTTOM` 静态（底边锚定不再需要）

**新增：**
- 常量：`DEFAULT_EXPANDED_W = 400.0`、`DEFAULT_EXPANDED_H = 300.0`、`MIN_EXPANDED_W = 300.0`、`MIN_EXPANDED_H = 200.0`
- 静态 `EXPANDED_SIZE: Mutex<(f64, f64)>`，初始 `(400, 300)`（逻辑像素）
- `pub fn set_expanded_size(w: f64, h: f64)` — 供启动时从 settings 载入
- `pub fn get_popup_rect(app_handle) -> Option<(f64,f64,f64,f64)>`：返回当前**内容区** rect（逻辑坐标 x,y,w,h，即已减去 SHADOW_MARGIN）。前端在 pointerdown 时取一次作为拖拽基准。
- `pub fn set_popup_rect(app_handle, x, y, w, h) -> (f64,f64,f64,f64)`：
  1. clamp 尺寸下限：`w ≥ MIN_W`、`h ≥ MIN_H`
  2. 取目标位置所在 monitor 可视区（`get_monitor_info_at`）
  3. clamp rect 到可视区内（位置 + 尺寸都不越界）
  4. 存 `EXPANDED_SIZE = (w, h)`（只存尺寸），`SetWindowPos` 应用（含 `SHADOW_MARGIN` 偏移，位置+尺寸原子设置）
  5. 返回 clamp 后的 `(x,y,w,h)`，供前端下一帧基准同步 + 尺寸持久化

**修改：**
- `compute_expanded_position(app_handle, w, h)` — 改为接收宽高两参；宽度不再从输入框推导，直接用传入值。定位逻辑（尝试选区上方 → 下方 → clamp 屏幕）保持。
- `expand_popup()` / `expand_popup_streaming()` — 都读 `EXPANDED_SIZE` 得到 `(w, h)`，调 `compute_expanded_position(w, h)` + `apply_expanded_layout`。两者合并为同一路径（不再区分估算 vs 固定高度）。
- `apply_expanded_layout` — 去掉 `POPUP_BOTTOM` 存储。

### 后端 `src-tauri/src/lib.rs`

- `Settings` 新增字段：
  ```rust
  #[serde(default = "default_popup_width")]  pub popup_width: f64,   // 400
  #[serde(default = "default_popup_height")] pub popup_height: f64,  // 300
  ```
  `Default` impl 中补 `popup_width: 400.0, popup_height: 300.0`。
- 启动时（settings 载入后）调 `overlay::set_expanded_size(s.popup_width, s.popup_height)`。
- 删除 `resize_popup_content` 命令，新增 `get_popup_rect` 和 `set_popup_rect`：
  ```rust
  #[tauri::command]
  fn get_popup_rect(app) -> Option<(f64,f64,f64,f64)> {
      overlay::get_popup_rect(&app)
  }

  #[tauri::command]
  async fn set_popup_rect(app, state, x: f64, y: f64, width: f64, height: f64)
      -> Result<(f64,f64,f64,f64), String> {
      let rect = overlay::set_popup_rect(&app, x, y, width, height);
      // 持久化 clamp 后的尺寸（只存宽高）
      let mut s = state.settings.lock().clone();
      s.popup_width = rect.2; s.popup_height = rect.3;
      s.save().ok();
      *state.settings.lock() = s;
      Ok(rect)
  }
  ```
  更新 `invoke_handler` 注册。

### 前端 `src/components/Popup.tsx`

**删除：**
- 「Resize popup to fit content」的 effect（约 406–441 行）
- `contentRef` 上仅为测量用的逻辑、`hasResized`、`sizeLockedForRegenerate` ref（确认无其他用途后移除；`resetState` 中相应清理也删）

**新增 resize 把手：**
- 一个 `<ResizeHandle>`，绝对定位于卡片右下角（`bottom-2 right-2` 区域，在 SHADOW_MARGIN 内），显示斜纹/三角，`cursor: nwse-resize`。
- 交互：
  - `onPointerDown`：`setPointerCapture`，记录起始 `clientX/Y` 与当前窗口尺寸（`window.innerWidth/innerHeight`，即内容尺寸）
  - `onPointerMove`：`newW = startW + (e.clientX - startX)`、`newH = startH + (e.clientY - startY)`，节流（`requestAnimationFrame` 合并）后 `invoke("resize_popup_window", { width: newW, height: newH })`
  - `onPointerUp`：`releasePointerCapture`
- 在 expanded / loading / streaming / error 四个卡片内都渲染此把手。

**内容溢出**：内容区保持 `flex-1 min-h-0 overflow-auto`，无需改。

## 测试 / 验收

- 首次触发 popup（无历史尺寸）为 400×300。
- 拖右下角能同时改宽高；松手后尺寸生效。
- 关闭 popup 再触发，尺寸保持上次值；重启 app 后仍保持。
- popup 贴近屏幕右/下边缘时，往大拖被 clamp，不越出屏幕。
- 内容超出窗口时内部滚动，不再自动改窗口尺寸。
- 拖到最小 300×200 无法再缩小。
- `cargo test` 通过；`tsc --noEmit` 通过。
