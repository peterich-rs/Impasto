import { advanceFrames, assert, fire, screen, test, trace } from "impasto:test";
import { createSignal, For } from "solid-js";
import { scene } from "@impasto/solid-renderer";
import {
  Button, Column, Field, Grid, Image, RepaintBoundary, Role, Row, Stack, Tabs, Text,
  View, VirtualList, setDensity, useTheme,
} from "@impasto/solid-ui";

test("native theme length tokens update static nodes in place", async () => {
  function Card() {
    const theme = useTheme();
    return <View accessibilityLabel="Token card" style={{
      width: theme().space.l,
      height: theme().fontSize.body,
      backgroundColor: theme().color.surface,
      borderRadius: theme().shape.m,
    }} />;
  }
  await screen.mount(Card);
  const node = await screen.byLabel("Token card");
  assert.equal(screen.rect(node).w, 16);
  const themeChanged = await trace(async () => {
    setDensity("comfortable");
    await advanceFrames();
  });
  assert.ok(themeChanged.styleResolves > 0);
  assert.equal(themeChanged.ffiBatches, 0);
  assert.ok(screen.rect(node).w > 16);
  assert.equal(await screen.byLabel("Token card"), node);
  setDensity("regular");
});

test("template refs see children and run in post-order", async () => {
  const refs: string[] = [];
  await screen.mount(() => <View ref={node => {
    if (node) {
      assert.ok(scene.firstChild(node));
      refs.push("parent");
    }
  }}>
    <Image alt="Static image" intrinsicSize={{ width: 20, height: 16 }} ref={node => {
      if (node) refs.push("image");
    }} />
    <View>{() => "dynamic"}</View>
  </View>);
  assert.deepEqual(refs, ["image", "parent"]);
  const image = await screen.byRole("image", { name: "Static image" });
  assert.equal(screen.rect(image).h, 16);
});

test("a direct keyed list reorders reused fragments around duplicate keys", async () => {
  const [items, setItems] = createSignal([{ key: 1, label: "A" }, { key: 2, label: "B" }, { key: 1, label: "C" }]);
  let list = 0;
  await screen.mount(() => <View ref={node => { list = node ?? 0; }}>
    <Text>Start</Text>
    <For each={items()} keyed={item => item.key} fallback="Empty">
      {item => <Text>{item().label}</Text>}
    </For>
    <Text>End</Text>
  </View>);
  const [a, b, c] = await Promise.all(["A", "B", "C"].map(screen.byText));
  const change = await trace(async () => {
    setItems([items()[1], { key: 1, label: "First" }, { key: 1, label: "Second" }]);
    await advanceFrames();
  });
  assert.equal(change.createdNodes, 0);
  assert.equal(change.ffiBatches, 1);
  assert.equal(scene.firstChild(list), await screen.byText("Start"));
  assert.equal(scene.nextSibling(scene.firstChild(list)), b);
  assert.equal(scene.nextSibling(b), a);
  assert.equal(scene.nextSibling(a), c);
  assert.equal(scene.nextSibling(c), await screen.byText("End"));
  assert.equal(await screen.byText("Second"), c);
  setItems([]);
  await advanceFrames();
  assert.ok(await screen.byText("Empty"));
});

test("a 200-row reversal keeps the list in a single native batch", async () => {
  const [items, setItems] = createSignal(Array.from({ length: 200 }, (_, index) => index));
  let parent = 0;
  await screen.mount(() => <View ref={node => { parent = node ?? 0; }}>
    <For each={items()}>{item => <Text>{item}</Text>}</For>
  </View>);
  const first = scene.firstChild(parent);
  let last = first;
  for (let index = 1; index < 200; index++) last = scene.nextSibling(last);
  assert.ok(last);
  const change = await trace(async () => {
    setItems(items().slice().reverse());
    await advanceFrames();
  });
  assert.equal(change.ffiBatches, 1);
  assert.equal(change.createdNodes, 0);
  assert.equal(scene.firstChild(parent), last);
  assert.equal(scene.nextSibling(first), 0);
});

test("compiled layout preserves component defaults, classes and text identity", async () => {
  const [wide, setWide] = createSignal(false);
  const [label, setLabel] = createSignal<string | undefined>("Change width");
  const [width, setWidth] = createSignal<number | undefined>(40);
  await screen.mount(() => (
    <Column style={{ width: 200 }}>
      <Row role={Role.Button} accessibilityLabel={label()} onClick={() => {
        setWide(true);
        setLabel("Wider");
        setWidth(undefined);
      }}>
        <View accessibilityLabel="Measured"
          class={wide() ? "w-[60px]" : "w-[20px]"}
          classList={{ "w-[64px]": wide() }}
          style={{ height: 20, width: width() }} />
        <Grid accessibilityLabel="Grid" style={{ width: 20, height: 20 }}>
          <Stack><RepaintBoundary><Text>Price: {wide() ? 2 : 1}</Text></RepaintBoundary></Stack>
        </Grid>
      </Row>
      <Row class="flex-col">
        <View accessibilityLabel="Top" style={{ width: 10, height: 10 }} />
        <View accessibilityLabel="Bottom" style={{ width: 10, height: 10 }} />
      </Row>
    </Column>
  ));
  const target = await screen.byLabel("Measured");
  const grid = await screen.byLabel("Grid");
  const labelNode = await screen.byText("Price: 1");
  assert.equal(screen.rect(target).w, 40);
  assert.ok(screen.rect(grid).x >= screen.rect(target).x + 40);
  assert.ok(screen.rect(await screen.byLabel("Bottom")).y > screen.rect(await screen.byLabel("Top")).y);
  const button = await screen.byRole("button", { name: "Change width" });
  const changes = await trace(() => fire.click(button));
  assert.equal(await screen.byRole("button", { name: "Wider" }), button);
  assert.equal(await screen.byText("Price: 2"), labelNode);
  assert.equal(screen.rect(target).w, 64);
  assert.equal(changes.createdNodes, 0);
});

test("a mixed shorthand restores the base value when a longhand becomes absent", async () => {
  const [left, setLeft] = createSignal<number | undefined>(30);
  await screen.mount(() => (
    <Row accessibilityLabel="Within padding" style={[{ padding: 10 }, { paddingLeft: left() }]}>
      <Text>Measurement</Text>
    </Row>
  ));
  const label = await screen.byText("Measurement");
  const container = await screen.byLabel("Within padding");
  assert.equal(screen.rect(label).x - screen.rect(container).x, 30);
  setLeft(undefined);
  await advanceFrames(1);
  assert.equal(screen.rect(label).x - screen.rect(container).x, 10);
});

const budgets = [
  ["Button", { createdNodes: 2, ffiCalls: 44, styleResolves: 10 }, () => <Button label="Save" onClick={() => {}} />],
  ["Field", { createdNodes: 13, ffiCalls: 173, styleResolves: 48 }, () => <Field value="" placeholder="Name" />],
  ["VirtualList", { createdNodes: 104, ffiCalls: 373, styleResolves: 157 }, () => (
    <VirtualList
      items={Array.from({ length: 70 }, (_, i) => i)}
      itemKey={(item) => item}
      estimatedHeight={24}
      style={{ width: 160, height: 120 }}
    >
      {(item) => <Row style={{ height: 24 }}><Text>Row {item}</Text></Row>}
    </VirtualList>
  )],
  ["Tabs", { createdNodes: 7, ffiCalls: 119, styleResolves: 32 }, () => (
    <Tabs tabs={[{ id: "a", label: "A" }, { id: "b", label: "B" }]}
      selected="a" onSelect={() => {}} />
  )],
] as const;

for (const [name, limits, render] of budgets) {
  test(`${name} mount performance budget`, async () => {
    await screen.mount(() => <View />);
    const actual = await trace(async () => {
      await screen.mount(render);
      await advanceFrames(2);
    });
    assert.ok(actual.createdNodes <= limits.createdNodes,
      `${name} created ${actual.createdNodes} nodes. Limit ${limits.createdNodes}.`);
    assert.ok(actual.ffiCalls <= limits.ffiCalls,
      `${name} made ${actual.ffiCalls} FFI calls. Limit ${limits.ffiCalls}.`);
    assert.ok(actual.styleResolves <= limits.styleResolves,
      `${name} resolved ${actual.styleResolves} styles. Limit ${limits.styleResolves}.`);
  });
}
