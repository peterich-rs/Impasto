import { assert, screen, test, trace } from "impasto:test";
import { For, createSignal } from "solid-js";
import { Text, scene } from "@impasto/solid-renderer";
import { View } from "@impasto/solid-renderer/host";

test("moves keyed native children without recreating them", async () => {
  const a = { label: "Alpha" };
  const b = { label: "Beta" };
  const [items, setItems] = createSignal([a, b]);
  await screen.mount(() => <View><For each={items()}>{item => <Text>{item.label}</Text>}</For></View>);
  const alpha = await screen.byText("Alpha");
  const beta = await screen.byText("Beta");
  assert.equal(scene.nextSibling(alpha), beta);
  const stats = await trace(async () => {
    setItems([b, a]);
    await screen.advanceFrames();
  });
  assert.equal(stats.createdNodes, 0);
  assert.equal(scene.nextSibling(beta), alpha);
});

test("style and rich text edits use the real scene tree", async () => {
  const [color, setColor] = createSignal("#111111");
  await screen.mount(() => (
    <View style={{ width: "50%", paddingLeft: 4 }}>
      <Text style={{ color: color(), fontSize: 16 }}>Hi</Text>
      <Text runs={[{ text: "Im", color: "#112233", gradient: {
        kind: "linear", from: [0, 0], to: [1, 0],
        stops: [{ offset: 0, color: "#111111" }, { offset: 1, color: [255, 0, 0, 255] }],
      }}]} />
    </View>
  ));
  const text = await screen.byText("Hi");
  assert.equal(screen.rect(scene.parent(text)).w, 180);
  assert.throws(() => scene.setProp(text, "w", "50%"), /unknown JSX property/);
  const stats = await trace(async () => {
    setColor("#ffffff");
    await screen.advanceFrames();
  });
  assert.equal(stats.createdNodes, 0);
  assert.equal(await screen.byText("Hi"), text);
  assert.ok(await screen.byText("Im"));
});

test("template bindings keep static siblings and their positions across updates", async () => {
  const [label, setLabel] = createSignal("Before");
  const [visible, setVisible] = createSignal(false);
  const [width, setWidth] = createSignal(80);
  const initial = await trace(async () => {
    await screen.mount(() => (
      <View>
        {label()}
        <View style={{ width: width(), height: 20 }}>Fixed</View>
        {visible() && <View>Middle</View>}
        <View>Last</View>
      </View>
    ));
  });
  assert.ok(initial.ffiBatches > 0);
  const first = await screen.byText("Before");
  const fixed = scene.parent(await screen.byText("Fixed"));
  const last = scene.parent(await screen.byText("Last"));
  assert.equal(scene.nextSibling(first), fixed);
  assert.equal(scene.nextSibling(fixed), last);
  assert.equal(screen.rect(fixed).w, 80);
  setLabel("After");
  setWidth(120);
  setVisible(true);
  await screen.advanceFrames();
  const middle = scene.parent(await screen.byText("Middle"));
  assert.equal(await screen.byText("After"), first);
  assert.equal(scene.nextSibling(first), fixed);
  assert.equal(scene.nextSibling(fixed), middle);
  assert.equal(scene.nextSibling(middle), last);
  assert.equal(screen.rect(fixed).w, 120);
  setVisible(false);
  await screen.advanceFrames();
  assert.equal(scene.nextSibling(fixed), last);
});
