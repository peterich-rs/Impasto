# 前置改造 5：macOS 桌面端自举底座改造方案

> **适用范围与前置约束**
> - 本文档细化支撑 `examples/devtools-desktop` 原生自举所需的 5 项桌面底座能力：**窗口配置化与尺寸记忆、系统等宽字体族注册、鼠标光标形态协议（`NSCursor`）、原生文件面板大文件补强、标准 macOS 菜单栏（`NSMenu`）**。
> - **跨平台兼容**：所有新增风格、光标与窗口配置均保证 iOS / Android 编译与运行零回归（非桌面平台安全忽略或默认空实现）。
> - **生成物约束**：在 `crates/engine/src/style/` 增加样式键后，必须执行 `just gen` 与 `just style-check`，禁止手改 `packages/solid-renderer/src/style.gen.ts` 与 [style-utilities.gen.md](style-utilities.gen.md)。
> - **Code & Lint 约束**：macOS crate 需在 macOS 主机通过 `cargo clippy -p embedder-macos --all-targets -- -D warnings`；所有新跨接 `objc2` 调用须标注清晰 `SAFETY` 注释并仅在主线程执行。

---

## 1. 改造目标与文件清单

| 改造项 | 涉及文件 | 核心内容 |
|---|---|---|
| 1. 窗口配置化 | [cli/src/app.rs](../crates/cli/src/app.rs) · [cli/src/build.rs](../crates/cli/src/build.rs) · [macos/src/lib.rs](../crates/embedder/macos/src/lib.rs) · [macos/src/app.rs](../crates/embedder/macos/src/app.rs) | `app.toml` `[macos.window]` 配置透传至 `Info.plist` + CLI 参数覆写 + `NSWindow` 尺寸/最小约束/记忆生效 |
| 2. 等宽字体族 | [macos/src/app.rs](../crates/embedder/macos/src/app.rs) | `MacServices::extra_fonts()` 注册系统等宽字体为 `"Monospace"`（对齐 [fonts.rs](../crates/raster/src/fonts.rs)） |
| 3. 光标形态协议 | [style/property.rs](../crates/engine/src/style/property.rs) · `tree/mod.rs` · `tree/hit.rs` · `session/mod.rs` · `embedder/common/src/lib.rs` · `macos/Cargo.toml` · `macos/src/app.rs` | `Key::Cursor` + `CursorStyle` → `Services::set_cursor` → `NSCursor` |
| 4. 原生文件面板补强 | [file-system/rust/src/lib.rs](../packages/file-system/rust/src/lib.rs) · [FileSystemPlugin.swift](../packages/file-system/apple/Sources/ImpastoFileSystem/FileSystemPlugin.swift) | 修复 `.pftrace`/`.impasto-session` 等未注册 UTI 报错；支持保存后缀过滤与大文件只读访问 |
| 5. 标准菜单栏与快捷键 | [macos/Cargo.toml](../crates/embedder/macos/Cargo.toml) · [macos/src/app.rs](../crates/embedder/macos/src/app.rs) | 安装 `NSMenu`（App/Edit/Window），协调系统快捷键与引擎 `keyDown:` 分流 |

---

## 2. 项一：窗口配置化与尺寸记忆

当前 [crates/embedder/macos/src/app.rs](../crates/embedder/macos/src/app.rs#L39-L40) 将窗口死锁在 `CONTENT_WIDTH = 360.0`、`CONTENT_HEIGHT = 640.0`（并在 `finish_launching()` 硬编码使用这两个常量设置初始 `metrics`）。

### 2.1 `app.toml` 与 CLI 配置模型

在 [crates/cli/src/app.rs](../crates/cli/src/app.rs#L40) 扩展 `MacConfig`：

```rust
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MacWindowConfig {
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub min_width: Option<f64>,
    pub min_height: Option<f64>,
    pub title: Option<String>,
    pub autosave_name: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MacConfig {
    pub usage_descriptions: BTreeMap<String, String>,
    pub entitlements: BTreeMap<String, bool>,
    pub window: Option<MacWindowConfig>,
}
```

- **合法性校验**：在 `MacConfig::validate()` 校验 `width`/`height` 处于 `200.0..=8192.0` 有限正数区间，且 `min_width <= width`、`min_height <= height`；`autosave_name` 仅允许字母数字和 `._-`。
- **构建透传（[crates/cli/src/build.rs](../crates/cli/src/build.rs)）**：在构建 macOS Runner 时将配置的窗口参数写入生成 `.app` 的 `Info.plist`（键名：`ImpastoWindowWidth`、`ImpastoWindowHeight`、`ImpastoWindowMinWidth`、`ImpastoWindowMinHeight`、`ImpastoWindowTitle`、`ImpastoWindowAutosaveName`）。

### 2.2 macOS 宿主读取与 `NSWindow` 应用

1. **读取优先级（[macos/src/lib.rs](../crates/embedder/macos/src/lib.rs)）**：CLI 启动参数（`--width`、`--height`、`--min-width`、`--min-height`） > `NSBundle.mainBundle().infoDictionary()` > 默认值（`360.0 × 640.0`，保持原有移动尺寸预览示例行为不变）。
2. **`NSWindow` 配置与布局修正（[macos/src/app.rs](../crates/embedder/macos/src/app.rs)）**：
   - 按配置的 `(width, height)` 创建 `CGRect`；
   - 若设置 `(min_width, min_height)`，调用 `window.setContentMinSize(CGSize::new(min_w, min_h))`；
   - 若设置 `autosave_name`，调用 `window.setFrameAutosaveName(&NSString::from_str(name))`；注意必须在 `setFrameAutosaveName` 之后再读取当前 `window.contentView().bounds().size`（因为 macOS 会把历史保存的窗口尺寸恢复到 `window` 上）；
   - 修复 `finish_launching()` 第 689-693 行：改用实际 `contentView` bounds 尺寸而非硬编码常量初始化 `CAMetalLayer` 与 `SurfaceReady` 的 `metrics`。

---

## 3. 项二：注册系统等宽字体族（`"Monospace"`）

经核实，[crates/raster/src/fonts.rs:246-258](../crates/raster/src/fonts.rs#L246-L258) 已经约定好通用的等宽字体族映射规则：

```rust
let generic = match family {
    "Sans" => Some(GenericFamily::SansSerif),
    "Serif" => Some(GenericFamily::Serif),
    "Monospace" => Some(GenericFamily::Monospace),
    _ => None,
};
```

当前 [crates/embedder/macos/src/app.rs:886-900](../crates/embedder/macos/src/app.rs#L886-L900) 的 `MacServices::extra_fonts()` 只返回了 `"SansCJK"`。改造方案：

- 在 `extra_fonts()` 中同时探测系统等宽字体文件（已验证存在：`/System/Library/Fonts/SFNSMono.ttf` → `/System/Library/Fonts/Menlo.ttc` → `/System/Library/Fonts/Monaco.ttf`），读取首个非空字体文件并以 **`"Monospace".to_string()`** 作为族名一并返回（同时保留 `"SansCJK"`）。
- 效果：DevTools 中所有设置 `fontFamily: "Monospace"` 或触发通用等宽回退的文本均直接命中 `SF Mono` / `Menlo`，并且自动联动系统字重与 Retina 亚像素光栅化。

---

## 4. 项三：鼠标光标形态协议（`CursorStyle` → `NSCursor`）

### 4.1 `crates/engine` 样式与命中扩展

1. **新增样式键（[style/property.rs](../crates/engine/src/style/property.rs)）**：
   - 新增 `Key::Cursor = 111`（`name = "cursor"`），`impact()` 返回 **`Impact::HIT`**（变更鼠标光标形状既不触发 Taffy Layout，也不需要重新录制 `LayerTree`）。
   - 定义 `CursorStyle` 枚举与对应 CSS 值解析：
     `Auto` | `Default` | `Pointer` | `Text` | `ColResize` | `RowResize` | `Grab` | `Grabbing` | `Move` | `NotAllowed` | `Crosshair`。
   - **必须执行生成校验**：运行 `just gen && just style-check`，自动同步更新 `packages/solid-renderer/src/style.gen.ts` 与 [style-utilities.gen.md](style-utilities.gen.md)。
2. **稀疏存储与祖先继承查找**：
   - 在 `Tree` 新增稀疏表 `cursors: HashMap<NodeId, CursorStyle>`（普通节点不增加任何字段体积，节点 `retire` 时清理）；
   - 当指针发生移动（`Phase::Move`）或帧结束命中树可能变动时，沿最近命中链路（`HitPath` 从 leaf 到 root）查找首个非 `CursorStyle::Auto` 的节点光标，若全链均为 `Auto` 则回落 `CursorStyle::Default`；仅当有效光标与上一帧/上次发送值不同步时才向宿主通知（去重防抖）。

### 4.2 宿主与 `embedder-macos` 接入

1. **Session 与 Services 桥接**：
   - 在 [crates/engine/src/session/mod.rs](../crates/engine/src/session/mod.rs) `Session` trait 新增 `fn take_cursor(&mut self) -> Option<crate::style::CursorStyle> { None }`；
   - 在 [crates/embedder/common/src/lib.rs](../crates/embedder/common/src/lib.rs) `Services` trait 新增 `fn set_cursor(&self, _cursor: engine::style::CursorStyle) {}`（iOS / Android 使用默认空实现）；
   - 在 [ui_frame.rs](../crates/embedder/common/src/host/ui_frame.rs) 处理完指针输入或帧渲染后检查 `session.take_cursor()` 并通知 `runtime.services.borrow().set_cursor(cursor)`。
2. **macOS `NSCursor` 映射（[macos/Cargo.toml](../crates/embedder/macos/Cargo.toml) & [app.rs](../crates/embedder/macos/src/app.rs)）**：
   - 在 `Cargo.toml` 为 `objc2-app-kit` 增加 `"NSCursor"` feature；
   - `MacServices::set_cursor` 通过 `on_main(...)` 在主线程映射并调用 `.set()`：
     - `Auto` / `Default` → `NSCursor::arrowCursor()`
     - `Pointer` → `NSCursor::pointingHandCursor()`
     - `Text` → `NSCursor::IBeamCursor()`
     - `ColResize` → `NSCursor::resizeLeftRightCursor()`
     - `RowResize` → `NSCursor::resizeUpDownCursor()`
     - `Grab` → `NSCursor::openHandCursor()`
     - `Grabbing` → `NSCursor::closedHandCursor()`
     - `Crosshair` → `NSCursor::crosshairCursor()`
     - `NotAllowed` → `NSCursor::operationNotAllowedCursor()`
     - `Move` → 系统移动或 `arrowCursor` 回退；
   - 在 `ContentView::mouse_exited` 主动恢复 `NSCursor::arrowCursor()`，防止光标离开 DevTools 窗口后残留拖拽形态。

---

## 5. 项四：原生文件打开/保存面板补强

经核实，[packages/file-system/apple/Sources/ImpastoFileSystem/FileSystemPlugin.swift](../packages/file-system/apple/Sources/ImpastoFileSystem/FileSystemPlugin.swift) 已经实现了 macOS `NSOpenPanel`（`pickFiles`/`pickDirectory`）与 `NSSavePanel`（`saveFile`），但直接用于 DevTools 时存在 3 个具体卡点需要补强：

1. **未注册自定义后缀报错问题**：
   - 现状第 104 行：`UTType(filenameExtension: ...)` 对 `.pftrace`、`.impasto-session`、`.impsym` 等未在系统注册的后缀会返回 `nil`，导致直接抛出 `fileReadUnsupportedScheme` 异常打断打开窗口！
   - 改造：`Self.types()` 遇到 `UTType(filenameExtension:)` 返回 `nil` 时安全回退为 `.data`（或使用 `UTType(exportedAs: ...)` / 放开 `panel.allowsOtherFileTypes = true`）。
2. **`save_file` 扩展名提示**：
   - 在 [packages/file-system/rust/src/lib.rs:230](../packages/file-system/rust/src/lib.rs#L230) `SaveFileOptions` 增加 `allowed_extensions: Option<Vec<String>>`（同步更新 `SaveOptions` Swift 解码），保存 `.pftrace` / `.impasto-session` / `.json` 时允许用户切换或自动补全扩展名。
3. **大文件免拷贝或及时清理策略**：
   - 现状 `pickFiles` 会将选中文件全量拷贝至应用沙盒 `Documents/Imports/<uuid>/` 再通过 JS `read()` 读取，对于动辄几十 MB 的 `.pftrace` / `.impasto-session` 会产生双倍 I/O；
   - 改造方案（二选一，推荐在 `@impasto/file-system` 扩展桌面端安全可选路径或 DevTools 专用读取路径）：
     - 方案 A（标准沙盒流式清理）：保持 `pickFiles` 返回后由 Rust Core（`@app/devtools-core`）直接通过 `AppPaths::document_dir()` + `relativePath` 以 `std::fs::File` 流式解析（不经过 JS 字节数组拷贝），解析完成后立即 `delete` 临时导入副本；
     - 方案 B（桌面无沙盒路径直读）：仅在桌面端未启用 App Sandbox 时允许 `pickFiles` 返回或供 Rust Core 使用只读句柄（需审慎评估安全边界，优先推荐 **方案 A：Rust Core 直接流式读 `Imports/` 并在加载后立即清理**，零安全口子且无 JS 内存拷贝）。

---

## 6. 项五：标准 macOS 菜单栏（`NSMenu`）与快捷键分流

1. **开启 Feature（[crates/embedder/macos/Cargo.toml](../crates/embedder/macos/Cargo.toml)）**：在 `objc2-app-kit` features 列表中新增 `"NSMenu"`、`"NSMenuItem"`。
2. **安装标准菜单（[crates/embedder/macos/src/app.rs](../crates/embedder/macos/src/app.rs) `app::run`）**：
   - 在 `app.run()` 启动前构建主 `NSMenu` 并调用 `app.setMainMenu(Some(&menu))`：
     - **App 菜单**（标题取自 `title`）：About、分隔线、Hide (`⌘H`)、Hide Others (`⌥⌘H`)、Show All、分隔线、Quit (`⌘Q` → `terminate:`)；
     - **Edit 菜单**：Cut (`⌘X` → `cut:`)、Copy (`⌘C` → `copy:`)、Paste (`⌘V` → `paste:`)、Select All (`⌘A` → `selectAll:`)；
     - **Window 菜单**：Minimize (`⌘M` → `performMiniaturize:`)、Zoom (`performZoom:`)、Close (`⌘W` → `performClose:`)。
3. **与 `ContentView::key_down` 的快捷键协调（关键细节）**：
   - 现状 [app.rs:223-235](../crates/embedder/macos/src/app.rs#L223-L235) 在 `keyDown:` 中拦截所有 `CONTROL | ALT | META` 组合键发往引擎 `InputEvent::Key`。
   - 安装 `NSMenu` 后，AppKit 会**先通过 `performKeyEquivalent:` 路由菜单栏已绑定的快捷键**（`⌘Q` / `⌘W` / `⌘M` / `⌘H` / `⌘C` / `⌘V` / `⌘X` / `⌘A`）；
   - 其中 Edit 菜单的 `copy:` / `cut:` / `paste:` / `selectAll:` 沿响应链发送到第一响应者 `ContentView`（`ContentView::perform_command` 已实现 `copy:`/`cut:`/`paste:`，仅需补齐 `selectAll:` 并透传至 `InputEvent::Key(Key::Char('a'), META)` 或专门选择命令）；
   - 未在菜单栏绑定的业务快捷键（`⌘K`、`⌘1~6`、`⌘F`、`⌘R`）直接进入 `ContentView::key_down`，经 `InputEvent::Key` 送往 Solid/TS 工作台分发。

---

## 7. 测试与验证清单

1. **样式代码生成校验**：
   ```sh
   just gen
   just style-check
   ```
2. **macOS 编译、Lint 与单元测试**：
   ```sh
   cargo clippy -p embedder-macos --all-targets -- -D warnings
   cargo test -p embedder-macos -p cli -p engine
   just boundaries
   ```
3. **首帧与桌面特性验证**：
   - 验证旧示例（默认不配 `[macos.window]`）使用 `IMPASTO_DUMP=/tmp/frame.png just host` 保持 `360×640` 无回归；
   - 使用 `--width 1360 --height 860` 参数与包含 `"Monospace"` 文本的验证页跑一次 `IMPASTO_DUMP` 截图，确认宽屏渲染、等宽对齐正常；
   - 使用带有 `.pftrace` 过滤条件的测试页面手动触发 macOS `pickFiles` / `saveFile` 与 `⌘Q` / `⌘W` / `⌘C` / `⌘V` 菜单响应。

## 8. 桌面示例入口

`impasto devtools` 打开 macOS 桌面示例并列出本机会话和已登记的 USB 设备。`impasto run --devtools` 将本次启动的目标传给示例。`impasto devtools --html` 保留原来的浏览器客户端。

[桌面示例](../examples/devtools-desktop/app.toml) 接入 Protocol v2，并展示已连接目标的窗口与最近帧信息。其 [Rust 模块](../examples/devtools-desktop/rust/src/lib.rs) 持有凭据、连接和符号表。选中文件交给 Rust 从暂存副本流式读取，读取完成或失败后清理副本。
