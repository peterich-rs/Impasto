import {
  Button, Column, Dialog, ErrorPage, Field, Image, Pressable, Recycler,
  Row, ScrollArea, Text, VirtualList, View,
  type NodeRef, type UIChildren,
} from "../src/index.tsx";

const ref: NodeRef = (node) => { if (node !== null) node.toFixed(); };
const content: UIChildren = [false, null, "text", 4, () => [<Text>row</Text>]];
<View ref={ref}>{content}</View>;
<Column><Row class="gap-2"><Text>标题</Text></Row></Column>;
<Button label="提交" onClick={() => {}} onPress={() => {}} onText={(text) => text.trim()}
  labelProps={{ style: [{ fontSize: 30 }, false, { fontSize: null }] }} />;
<Pressable ref={ref} disabled onWheel={(dx, dy) => dx + dy}>{content}</Pressable>;
<Field inputRef={ref} textProps={{ style: [{ fontSize: 30 }, undefined] }} />;
<Dialog open={false} onClose={() => {}} ref={ref} titleProps={{ class: "text-4xl" }} />;
<ScrollArea contentRef={ref}>{content}</ScrollArea>;
<Recycler each={[1]} keyOf={String} typeOf={() => "row"} render={() => content} />;
<VirtualList items={[1]} itemKey={String} estimatedHeight={20}
  recycle={{ typeOf: () => "row", render: () => content }} empty={() => content} />;
<ErrorPage message="失败" retry={() => {}} textProps={{ style: [{ color: "#000000" }] }} />;
<Image source="asset:a" intrinsicSize={{ width: 8, height: 8 }} ref={ref} />;

// @ts-expect-error public refs must accept cleanup with null
<View ref={(node: number) => node.toFixed()} />;
// @ts-expect-error removed shortcut
<Row gap={4} />;
// @ts-expect-error removed shortcut
<Button label="提交" onClick={() => {}} wide />;
// @ts-expect-error leaf images have no children
<Image source="asset:a">child</Image>;
// @ts-expect-error child style targets do not expose arbitrary events
<Field textProps={{ onClick: () => {} }} />;
// @ts-expect-error style fragments must use registered style fields
<Field textProps={{ style: [{ bogus: 1 }] }} />;
// @ts-expect-error contentProps is the only public scroll content target
<ScrollArea contentStyle={{ gap: 4 }} />;
// @ts-expect-error list renderers must return scene content
<Recycler each={[1]} keyOf={String} typeOf={() => "row"} render={() => ({ bad: 1 })} />;
