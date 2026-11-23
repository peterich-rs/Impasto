# 原生工具类

由 `style-types` 从属性注册表生成。类名使用 Tailwind 的常用词，默认值、单位和冲突顺序是本框架自己的。

数字间距 `p-4` 是 4 个间距步长，默认一步 4 点。`p-[4px]` 是 4 个场景点。`rem` 留到环境解析，不乘死 16。

同一 class 来源里，后写的同一长属性胜出。`p-4 pl-2` 只覆盖 left。变体优先级是 base、响应式、dark、hover、focus、active、disabled。

`class="flex"` 只写 display，不改方向。横排用 `flex-row`。

## 属性

- `width` (`box`): `SizeValue`
- `height` (`box`): `SizeValue`
- `minWidth` (`box`): `MinSize`
- `minHeight` (`box`): `MinSize`
- `maxWidth` (`box`): `MaxSize`
- `maxHeight` (`box`): `MaxSize`
- `flex` (`box`): `FlexValue`
- `flexGrow` (`box`): `number`
- `flexShrink` (`box`): `number`
- `flexBasis` (`box`): `BasisValue`
- `padding` (`box`): `Length`
- `paddingTop` (`box`): `Length`
- `paddingRight` (`box`): `Length`
- `paddingBottom` (`box`): `Length`
- `paddingLeft` (`box`): `Length`
- `paddingInlineStart` (`box`): `Length`
- `paddingInlineEnd` (`box`): `Length`
- `margin` (`box`): `MarginValue`
- `marginTop` (`box`): `MarginValue`
- `marginRight` (`box`): `MarginValue`
- `marginBottom` (`box`): `MarginValue`
- `marginLeft` (`box`): `MarginValue`
- `marginInlineStart` (`box`): `MarginValue`
- `marginInlineEnd` (`box`): `MarginValue`
- `gap` (`box`): `Length`
- `rowGap` (`box`): `Length`
- `columnGap` (`box`): `Length`
- `display` (`box`): `"flex" | "grid" | "block" | "flow-root" | "none"`
- `flexDirection` (`box`): `"row" | "column" | "row-reverse" | "column-reverse"`
- `flexWrap` (`box`): `"nowrap" | "wrap" | "wrap-reverse"`
- `alignItems` (`box`): `"normal" | "start" | "end" | "flex-start" | "flex-end" | "center" | "stretch" | "baseline"`
- `justifyContent` (`box`): `"start" | "end" | "flex-start" | "flex-end" | "center" | "stretch" | "space-between" | "space-around" | "space-evenly"`
- `alignSelf` (`box`): `"auto" | "normal" | "start" | "end" | "flex-start" | "flex-end" | "center" | "stretch" | "baseline"`
- `alignContent` (`box`): `"start" | "end" | "flex-start" | "flex-end" | "center" | "stretch" | "space-between" | "space-around" | "space-evenly"`
- `justifyItems` (`box`): `"normal" | "start" | "end" | "flex-start" | "flex-end" | "center" | "stretch" | "baseline"`
- `justifySelf` (`box`): `"auto" | "normal" | "start" | "end" | "flex-start" | "flex-end" | "center" | "stretch" | "baseline"`
- `position` (`box`): `"relative" | "absolute"`
- `inset` (`box`): `InsetValue`
- `top` (`box`): `InsetValue`
- `right` (`box`): `InsetValue`
- `bottom` (`box`): `InsetValue`
- `left` (`box`): `InsetValue`
- `insetInlineStart` (`box`): `InsetValue`
- `insetInlineEnd` (`box`): `InsetValue`
- `aspectRatio` (`box`): `number`
- `boxSizing` (`box`): `"border-box" | "content-box"`
- `gridTemplateColumns` (`box`): `readonly GridTrack[] | string`
- `gridTemplateRows` (`box`): `readonly GridTrack[] | string`
- `gridAutoColumns` (`box`): `readonly GridTrack[] | string`
- `gridAutoRows` (`box`): `readonly GridTrack[] | string`
- `gridAutoFlow` (`box`): `"row" | "column" | "row dense" | "column dense"`
- `gridColumnStart` (`box`): `GridLine`
- `gridColumnEnd` (`box`): `GridLine`
- `gridRowStart` (`box`): `GridLine`
- `gridRowEnd` (`box`): `GridLine`
- `overflow` (`box`): `"visible" | "clip" | "hidden"`
- `overflowX` (`box`): `"visible" | "clip" | "hidden"`
- `overflowY` (`box`): `"visible" | "clip" | "hidden"`
- `layoutDirection` (`box`): `Inherited<"ltr" | "rtl">`
- `zIndex` (`paint`): `number`
- `backgroundColor` (`paint`): `Color`
- `borderColor` (`paint`): `Color`
- `borderWidth` (`paint`): `Length`
- `border` (`paint`): `Length`
- `outlineWidth` (`paint`): `Length`
- `outlineColor` (`paint`): `Color`
- `borderRadius` (`paint`): `Length`
- `borderRadii` (`paint`): `readonly [number, number, number, number]`
- `borderCurve` (`paint`): `number`
- `opacity` (`paint`): `number`
- `translateX` (`paint`): `number`
- `translateY` (`paint`): `number`
- `rotate` (`paint`): `number`
- `scale` (`paint`): `number`
- `scaleX` (`paint`): `number`
- `scaleY` (`paint`): `number`
- `transformOriginX` (`paint`): `Length`
- `transformOriginY` (`paint`): `Length`
- `shadow` (`paint`): `ShadowStyle`
- `boxShadow` (`paint`): `readonly BoxShadowStyle[]`
- `gradient` (`paint`): `GradientStyle`
- `blur` (`paint`): `number`
- `backdrop` (`paint`): `number`
- `blend` (`paint`): `"src-over" | "multiply" | "screen"`
- `color` (`text`): `Inherited<Color>`
- `fontSize` (`text`): `Inherited<AbsoluteLength>`
- `fontWeight` (`text`): `Inherited<number>`
- `fontFamily` (`text`): `Inherited<string>`
- `textAlign` (`text`): `Inherited<"left" | "center" | "right">`
- `writingDirection` (`text`): `Inherited<"ltr" | "rtl">`
- `lineHeight` (`text`): `Inherited<number | "normal" | { points: number }>`
- `maxLines` (`text`): `number`
- `ellipsis` (`text`): `string`
- `softWrap` (`text`): `boolean`
- `textOverflow` (`text`): `"clip" | "ellipsis" | "visible"`
- `underline` (`text`): `boolean`
- `lineThrough` (`text`): `boolean`
- `objectFit` (`paint`): `"fill" | "contain" | "cover"`

## 类族

- display: `flex` `grid` `block` `flow-root` `hidden`
- 尺寸: `w-` `h-` `size-` `min-w-` `max-w-` `min-h-` `max-h-`，以及 `min` `max` `fit` `screen` `full` `auto`
- Flex: `flex-` `grow` `shrink` `basis-` `flex-row` `flex-col` `flex-wrap`
- 对齐: `items-` `self-` `justify-` `content-`
- 间距: `p-` `px-` `py-` `m-` `mx-` `my-` `gap-`，含负值和 `auto`
- 定位: `relative` `absolute` `inset-` `top-` `right-` `bottom-` `left-`
- Grid: `grid-cols-` `grid-rows-` `col-span-` `row-span-`
- 颜色: `bg-` `text-` `border-` `outline-`，主题 token、hex 和 `/透明度`
- 文字: `text-` `font-` `leading-` `text-left` `truncate` `underline` `line-through`
- 装饰: `rounded` `border` `outline` `shadow` `blur` `backdrop`
- 变换: `opacity-` `translate-` `rotate-` `scale-` `origin-` `z-`
- 图片: `object-fill` `object-contain` `object-cover`
- 变体: `sm` `md` `lg` `xl` `2xl` `dark` `hover` `focus` `active` `disabled`

不支持 `calc`、`var`、任意属性、`group`/`peer`、`container`、`important`、`fixed`、`sticky`、`subgrid`、`float` 和 `order`。未知 token 报错，不会当成 `auto`。
