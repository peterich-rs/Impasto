# 方向三设计方案：编译期强类型 i18n、跨模块深层常量折叠与紧凑矢量资产

本文档是 [`docs/native-toolchain-v2-roadmap.md`](native-toolchain-v2-roadmap.md) **方向三**的完整工程落地设计方案，聚焦三大编译期工程化与资产优化能力：**编译期强类型多语言（i18n）管线**、**通用 TypeScript 跨模块深层常量折叠（含热更新反向失效保证）**，以及**基于 `Path2D::from_svg` 的紧凑矢量图标与 `currentColor` 动态着色**。

适用模块：[`crates/bundler/src/{styles.rs,svg.rs,bundle.rs}`](../crates/bundler/src/lib.rs)、[`crates/engine/src/canvas/path.rs`](../crates/engine/src/canvas/path.rs)、[`crates/cli/src/sync.rs`](../crates/cli/src/sync.rs)、[`packages/i18n/`](../packages/i18n/src/index.ts) 与 [`packages/solid-ui/src/Icon.tsx`](../packages/solid-ui/src/Icon.tsx)。

---

## 1. 子方案 A：编译期强类型国际化（i18n）管线

### 1.1 现状缺口

当前 [`packages/i18n/src/index.ts`](../packages/i18n/src/index.ts) 仅提供基础数字、简单复数与日期格式化，[`packages/solid-ui/src/i18n.ts`](../packages/solid-ui/src/i18n.ts) 内置了 5 个控件词条。业务应用缺少标准化的多语言词条目录约定，面临三个痛点：
1. 词条键名拼写错误或插值参数遗漏（如词条含 `{shares}` 但调用未传）只能在运行期暴露为空白文本；
2. 新增词条漏翻某一种语言时无构建期门禁；
3. 将所有语言的完整 JSON 打入主包会造成不必要的内存与字节码体积浪费。

### 1.2 目录约定与词条语法

应用在根目录下可选放置 `locales/<bcp47>.json`（支持嵌套命名空间对象或扁平点分键，默认基准语言可在 `app.toml` 的 `[i18n] default_locale = "zh-CN"` 配置，未配置时以字典序首个文件或 `"en"` 为基准）：

```json
{
  "trade": {
    "submit": "买入 {symbol}",
    "shares_count": {
      "one": "{count} 股",
      "other": "{count} 股"
    }
  }
}
```

支持的插值语法：
- 普通命名占位符：`{symbol}`（推断类型为 `string | number`）；
- 数值/复数占位符：含 `{ one, other, zero? }` 复数分支的对象自动要求传入 `{ count: number }`（及分支内出现的其他占位符）。

### 1.3 编译期校验、类型生成与死词条消除

1. **构建与同步期静态门禁（新增 `crates/bundler/src/i18n.rs`）**：
   - **键集合对齐校验**：非基准语言文件若缺失基准语言的键，或包含基准语言不存在的孤立键，直接报出带文件路径与键名的构建诊断；
   - **占位符集合一致性校验**：同一键在不同语言（及同一语言的 `one`/`other` 分支）中提取的 `{param}` 集合必须一致（允许复数 `one` 分支省略 `{count}`，但不允许出现未知参数名）。
2. **自动生成强类型声明（`.impasto/types/i18n.d.ts`）**：
   - `impasto sync` 扫描 `locales/*.json` 生成精确重载映射：
     ```ts
     declare module "@impasto/i18n" {
       export interface ImpastoMessages {
         "trade.submit": { symbol: string | number };
         "trade.shares_count": { count: number };
       }
       export type MessageKey = keyof ImpastoMessages;
       export function t<K extends MessageKey>(
         key: K,
         ...args: ImpastoMessages[K] extends Record<string, never> ? [] : [params: ImpastoMessages[K]]
       ): string;
     }
     ```
   - 无参词条禁止多传参数，有参词条强制必传对应字段，`impasto analyze` 100% 静态拦截。
3. **编译期索引化与死词条消除（Dead-Message Elimination）**：
   - `ImpastoPlugin` 提供虚拟模块 `virtual:impasto-locales`。在 Release 链接阶段，扫描存活 AST 对 `@impasto/i18n` 的 `t("...")` 调用：
     - 若全部调用点均为静态字符串字面量键，**自动剔除所有未被引用的词条**，并将字符串键替换为紧凑整数下标 `t(14, params)`，省去打包产物中的长键名重复存储；
     - 若检测到动态键访问（`t(dynamicKey)`），则自动保留对应命名空间词表不做激进裁剪。
   - **响应式语言切换**：`@impasto/i18n` 的 `t()` 内部订阅 `theme.ts` / `scene.onSettings` 的 `locale` Signal，系统或应用内切换语言时，所有在 JSX 中调用 `{t("...")}` 的文本节点自动响应式更新。

---

## 2. 子方案 B：跨模块深层常量折叠与 Watch 反向失效机制

### 2.1 现有局限分析

审视 [`crates/bundler/src/styles.rs`](../crates/bundler/src/styles.rs) 的 `linked_constants` 与 `exported_constants` 实现，当前存在三处明显限制：
1. **仅支持带显式 `.ts`/`.tsx` 后缀的相对导入**：`import { ink } from "./colors"`（TypeScript 最常见的省略后缀写法）或目录入口 `import { SPACE } from "../design"` 因 `path.extension()` 为空被直接跳过；
2. **不支持桶文件重导出（Barrel Re-exports）**：`export { Palette } from "./palette"` 或 `export * from "./tokens"` 无法穿透解析；
3. **对象仅限单层 `Object.freeze`**：TypeScript 生态标准的 `export const Colors = { brand: { primary: "#2563eb" } } as const`（既无 `Object.freeze` 又是多层嵌套）全部无法折叠，导致引用这些常量的样式掉出 `StaticStyle` 池。

### 2.2 升级设计：模块解析器对齐 + 深层不可变证明 + 增量缓存安全

1. **复用路径解析规则定位导出模块**：
   - 将 `styles.rs` 中手写的 `parent.join(name)` 扩展为标准解析探测顺序：`exact` → `.ts` → `.tsx` → `/index.ts` → `/index.tsx`（同时支持解析 `@impasto/*` 内置包中的公开常量），并限制最大重导出跳数（`MAX_REEXPORT_DEPTH = 8`，使用 `HashSet<PathBuf>` 防止循环 `export *` 死循环）。
2. **支持 `as const` 与多层不可变字面量树**：
   - 在 `exported_constants` 的 AST 分析（在 TS 类型擦除前运行）中，识别以下三类深层不可变初始化器：
     - 原始类型字面量与纯字面量算术一元/二元常量表达式（如 `const UNIT = 4; export const GAP_M = UNIT * 3`）；
     - 带 `TSAsExpression` / `TSSatisfiesExpression` 且目标为 `const` 断言（`... as const`）的任意深度嵌套对象/数组字面量；
     - 递归调用 `Object.freeze(...)` 的嵌套字面量，或模块内无任何成员写入引用（通过 `Scoping` 验证所有引用仅做静态属性读取链）的未逃逸只读对象。
3. **关键正确性保障：`WatchLinker` 与 `ModuleCache` 的反向依赖失效（Invalidation Invariant）**：
   - **隐患场景**：假设 `Card.tsx` 写了 `import { PRIMARY } from "./colors"` 并用于 `<View style={{ backgroundColor: PRIMARY }} />`。`Card.tsx` 在 `transform` 时将 `PRIMARY` 内联进了静态样式池，Rolldown 摇树后 `colors.ts` 可能不再有运行时导出残留。此时用户在 `impasto run --watch` 下修改 `colors.ts` 中的色值，若不通知 `Card.tsx` 重新执行 `compile_inner`，页面颜色将不会更新！
   - **解决方案**：
     - `linked_constants` 返回 `(HashMap<SymbolId, Json>, BTreeSet<PathBuf>)`，后者记录本次常量折叠实际读取的外部源文件路径集合 `folded_deps`；
     - 在 `Linker` 中维护反向映射 `folded_dependents: BTreeMap<PathBuf, BTreeSet<PathBuf>>`；
     - 当 `WatchLinker::set_changes(changed)` 收到 `colors.ts` 变更时，自动将 `folded_dependents["colors.ts"]`（即 `Card.tsx`）并入本次 `ScanMode::Partial` 的重扫描集合，并使缓存失效——确保**跨文件编译期内联与毫秒级热更新 100% 一致，绝不产生陈旧常量**。

---

## 3. 子方案 C：紧凑矢量资产（`Path2D::from_svg`）与 `currentColor` 动态着色

### 3.1 现状冗余分析

查看 [`crates/bundler/src/svg.rs`](../crates/bundler/src/svg.rs) 与 [`crates/engine/src/canvas/path.rs`](../crates/engine/src/canvas/path.rs)：
- `engine::canvas::Path2D` **底层早已实现 `Path2D::from_svg(data: &str)`**（基于 `kurbo::BezPath::from_svg`），并在 QuickJS 构造函数 `new scene.Path2D(svgPathString)` 中暴露；
- 但 `bundler/src/svg.rs` 目前对每条路径生成几十行命令式 JS（`path0.moveTo(...); path0.bezierCurveTo(...); ctx.save(); ctx.transform(...)`），一个中等复杂度图标会膨胀出数 KB 的 JS 语句；
- 同时 `svg.rs` 将颜色写死为静态 `"rgba(...)"` 字符串，导致在按钮悬停、选中态、深浅色切换时，同一个 SVG 图标无法动态改变颜色，也无法响应常见图标库（Lucide / Heroicons / Material Symbols）广泛使用的 `stroke="currentColor"` / `fill="currentColor"`。

### 3.2 升级方案：构建期烘焙矩阵 + 紧凑路径字符串 + 着色参数化

1. **构建期预乘仿射变换并压缩 SVG Path 字符串**：
   - 在 `svg.rs` 遍历 `usvg::Node::Path` 时，直接将 `path.abs_transform()` 预乘到每个控制点坐标上（保留 2 位有效小数并去除尾随零，消除运行期每帧每次绘制调 `ctx.save() + ctx.transform(...) + ctx.restore()` 的状态栈开销）；
   - 将相邻同样式、同填充规则、无描边冲突的纯填充子路径合并为单条紧凑 SVG `d` 字符串（如 `"M12 4L20 12L12 20Z"`），模块顶层仅生成惰性单例：`let p0; ... (p0 ??= new scene.Path2D("M..."))`——**图标未上屏零 `BezPath` 内存占用，生成的 JS 代码体积缩减 70%~85%**。
2. **原生支持 `currentColor` 与 `draw(ctx, options)` 着色协议**：
   - 在 `roxmltree` 预扫描阶段记录使用 `currentColor` 的节点（`usvg` 默认将 `currentColor` 解析为 `usvg::Options::default()` 的黑色；通过设置唯一哨兵色或保留属性映射精确区分“作者显式写的黑色 `#000`”与“`currentColor`”）；
   - 升级导出的 SVG 模块签名（同步更新 [`crates/bundler/assets.d.ts`](../crates/bundler/assets.d.ts) 与 [`packages/solid-ui/src/Icon.tsx`](../packages/solid-ui/src/Icon.tsx)）：
     ```ts
     export interface VectorIconDrawOptions {
       color?: string | readonly [number, number, number, number];
       opacity?: number;
     }
     export interface VectorAsset {
       readonly width: number;
       readonly height: number;
       draw(ctx: CanvasRenderingContext2D, options?: VectorIconDrawOptions): void;
     }
     ```
   - `<Icon asset={CheckIcon} color={theme().color.accent} size={20} />` 自动按视口缩放 `width`/`height` 并将 `color` 传入 `draw(ctx, { color })` 替换所有 `currentColor` 填充与描边，无需准备多份不同颜色的 SVG 文件。
3. **支持线性与径向渐变绘制**：
   - 解除 `svg.rs` 对 `Paint::LinearGradient` 与 `Paint::RadialGradient` 的 `bail!` 限制，直接降级为 Impasto `CanvasRenderingContext2D` 已支持的 `ctx.createLinearGradient` / `ctx.createRadialGradient` + `addColorStop` 调用，让彩色徽章与插画类 SVG 也能直接零光栅失真渲染。

---

## 4. 实施步骤与验证门禁

| 步骤 | 改动范围 | 验证用例与门禁 |
|---|---|---|
| **Step 1：跨模块无后缀与深层常量折叠 + Watch 反向失效** | `styles.rs`、`bundle.rs`、`crates/testing/tests/hot.rs` | 测试无后缀导入 `./palette`、`index.ts` 重导出、`as const` 三层嵌套对象均成功进入 `StaticStyle` 池；新增 HMR 测试：修改 `palette.ts` 色值后，引用它的组件立即热刷新为新颜色。 |
| **Step 2：紧凑 SVG 编译、`currentColor` 与渐变支持** | `svg.rs`、`assets.d.ts`、`Icon.tsx`、`crates/testing/tests/assets.rs` | 对比 10 个标准 Lucide 图标编译前后 JS 字节数（目标下降 ≥ 70%）；使用 `headless-gpu` 快照验证旋转/缩放矩阵预乘、`currentColor` 变色及线性渐变像素准确。 |
| **Step 3：编译期强类型 i18n 管线** | `bundler/src/i18n.rs`、`sync.rs`、`packages/i18n` | 验证缺失词条、占位符不匹配在构建期报错；验证 `t()` 的 TS 参数类型检查；验证 Release 构建剔除未引用词条且运行期切换 `locale` 时界面文案即时刷新。 |
