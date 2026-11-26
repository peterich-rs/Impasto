import { View as HostView, Stack as HostStack } from "@impasto/solid-renderer/host";
import { batch, createRoot, createSignal } from "solid-js";
import { scene } from "@impasto/solid-renderer";
import { Button, Column, Image, Row, Text } from "@impasto/solid-ui";

const assert = (condition: unknown, message: string) => {
  if (!condition) throw new Error(message);
};

const root = <HostView style={{ width: 240, height: 640, alignItems: "start" }} />;

createRoot((dispose) => {
  const [width, setWidth] = createSignal(10);
  let clicks = 0;
  let refReads = 0;
  let mounted = 0;
  const ref = () => {
    refReads++;
    return (node: JSX.Element) => { mounted = node; };
  };

  function Probe(props: { onClick: () => void; onclick: number }) {
    assert(clicks === 0, "JSX must pass onClick as a callback");
    props.onClick();
    return <HostView {...{ style: { width: 1 } }} ref={ref()} style={{ width: props.onclick, height: 10 }}>
      {props.onclick > 10 && <HostView style={{ width: props.onclick, height: 1 }} />}
    </HostView>;
  }

  const node = <Probe {...{ onclick: 0 }} onClick={() => { clicks++; }} onclick={width()} />;
  scene.append(root, node);
  assert(clicks === 1, "component can invoke onClick");
  assert(mounted === node && refReads === 1, "JSX reads the ref factory once at creation");
  assert(scene.rect(node).w === 10 && scene.firstChild(node) === 0, "initial JSX width and child");
  batch(() => setWidth(20));
  const child = scene.firstChild(node);
  assert(
    scene.rect(node).w === 20 && child !== 0 && scene.rect(child).w === 20,
    "dynamic JSX prop and child update together",
  );
  assert(refReads === 1, "updating JSX props leaves the direct ref alone");
  dispose();
  scene.remove(root, node);
});

createRoot(() => {
  const page = (
    <Column style={{ width: 240, height: 320, layoutDirection: "ltr", gap: 8 }}>
      <Row style={{ gap: 8, justifyContent: "space-between" }}>
        <Text style={{ fontSize: 20 }}>自选</Text>
        <Button
          label="添加"
          variant="primary"
          onClick={() => {}}
          style={{ width: "100%", flexShrink: 1 }}
          labelProps={{ style: { fontWeight: 600 } }}
        />
      </Row>
      <Image
        source="asset:avatar"
        alt="用户头像"
        intrinsicSize={{ width: 8, height: 8 }}
        style={{ width: 32, height: 32, objectFit: "cover" }}
      />
    </Column>
  );
  scene.append(root, page);
  assert(scene.rect(page).w === 240, "column keeps its author width");
  const stack = <HostStack style={{ width: 20, height: 20 }} />;
  let rejected = false;
  try {
    scene.setStyle(stack, { gridTemplateColumns: [{ fr: 1 }] });
  } catch {
    rejected = true;
  }
  assert(rejected, "stack track override must fail");
  scene.setStyle(stack, { display: "none" });
  const rows = <HostView />;
  const rowDefaults = scene.createStyleSource(rows, "component");
  scene.setStyleSource(rowDefaults, { width: 10, height: 10 });
  scene.append(root, rows);
  scene.setStyleSurface(rows, "list-content");
  scene.setStyle(rows, { color: "#112233" });
  rejected = false;
  try {
    scene.setStyle(rows, { width: 40 });
  } catch {
    rejected = true;
  }
  assert(rejected, "list content must reject width");
  assert(scene.rect(rows).w === 10, "rejected width must stay");
  const picture = (
    <Image source="asset:a" intrinsicSize={{ width: 8, height: 8 }} style={{ width: 32 }} />
  );
  scene.append(root, picture);
  scene.setImage(picture, "asset:b");
  assert(scene.rect(picture).w === 32, "source update keeps author width");
});

// Detached overlay targets must keep tracking the same props object.
import { createNodeRef, createStyleTarget } from "../../../../../packages/solid-ui/src/target.ts";
import { ErrorPage, Field, Pressable, Recycler, View, installThemePort } from "@impasto/solid-ui";

createRoot((dispose) => {
  const [width, setWidth] = createSignal(10);
  const [classes, setClasses] = createSignal("w-8");
  const [author, setAuthor] = createSignal(false);
  const target = createStyleTarget(() => ({
    get class() { return classes(); },
    get classList() { return { "h-4": width() === 30 }; },
    get style() { return author() ? [{ width: width() }, false] : undefined; },
  }), () => ({ width: 10 }));
  batch(() => {});
  const first = <View />;
  scene.append(root, first);
  target.attach(first);
  assert(scene.rect(first).w === 32, "author class overrides component defaults");
  batch(() => setAuthor(true));
  assert(scene.rect(first).w === 10, "inline overrides classes");
  batch(() => setWidth(30));
  assert(scene.rect(first).w === 30, "delayed target tracks inline getter");
  assert(scene.rect(first).h === 16, "delayed target tracks classList getter");
  target.detach();
  batch(() => { setAuthor(false); setClasses("w-12"); });
  const second = <View />;
  scene.append(root, second);
  target.attach(second);
  assert(scene.rect(second).w === 48, "reopened target gets current class snapshot");
  dispose();
});

let disposeRef = () => {};
let nullsA = 0, nullsB = 0, mountsA = 0, mountsB = 0;
const refA = (id: number | null) => id === null ? nullsA++ : mountsA++;
const refB = (id: number | null) => id === null ? nullsB++ : mountsB++;
let setRef = (_ref: () => typeof refA) => {};
let panelRef: ReturnType<typeof createNodeRef>;
createRoot((done) => {
  disposeRef = done;
  const [ref, write] = createSignal(refA);
  setRef = write;
  panelRef = createNodeRef(ref);
});
batch(() => {});
const refNode = <View />;
panelRef!.attach(refNode);
panelRef!.attach(refNode);
batch(() => setRef(() => refB));
assert(mountsA === 1 && nullsA === 1 && mountsB === 1, "dynamic panel ref replaces exactly once");
panelRef!.clear();
panelRef!.clear();
disposeRef();
assert(nullsA === 1 && nullsB === 1, "clear and owner disposal revoke each mount once");

createRoot((dispose) => {
  let mounts = 0, clears = 0;
  const ref = createNodeRef(() => id => id === null ? clears++ : mounts++);
  ref.attach(<View />);
  ref.clear();
  ref.attach(<View />);
  ref.clear();
  dispose();
  assert(mounts === 2 && clears === 2, "reopening pairs each ref lifecycle");
});

createRoot(() => {
  const [disabled, setDisabled] = createSignal(true);
  const control = <Pressable disabled={disabled()} class="disabled:w-8" />;
  scene.append(root, control);
  assert(scene.rect(control).w === 32, "Pressable disabled variant reads native semantics");
  batch(() => setDisabled(false));
  assert(scene.rect(control).w === 0, "Pressable removes native disabled state");
});

// A single slot shell owns arbitrary scene children, including accessors.
createRoot((dispose) => {
  const rows = <Recycler each={[1]} keyOf={String} typeOf={() => "row"}
    render={() => [<View style={{ width: 7, height: 5 }} />, () => <View style={{ width: 9, height: 6 }} />]} />;
  scene.append(root, rows);
  batch(() => {});
  scene.rect(rows);
  const slot = scene.firstChild(rows);
  assert(slot !== 0, "Recycler creates a real row owner");
  assert(scene.nextSibling(scene.firstChild(slot)) !== 0, "slot normalizes multiple children");
  assert(scene.rect(slot).h === 11, "row owner measures all normalized children");
  dispose();
});

// These leaves are kept in the final frame so Rust checks their DrawList sizes.
createRoot(() => {
  const page = <Column style={{ width: 240, height: 640 }}>
    <Field value="field-array" textProps={{ style: [{ fontSize: 30 }] }} />
    <Field value="field-class" textProps={{ class: "text-4xl" }} />
    <Field value="field-reset" textProps={{ style: [{ fontSize: 30 }, { fontSize: null }] }} />
    <Field value="field-error" error="error-array" errorProps={{ style: [{ fontSize: 26 }] }} />
    <ErrorPage message="error-page-array" retry={() => {}}
      textProps={{ style: [{ fontSize: 28 }] }} actionProps={{ style: [{ width: 80 }] }} />
  </Column>;
  scene.append(root, page);
  scene.rect(page);
});


// Replacing the theme port must synchronize direction before any themed control is used.
installThemePort({
  settings: () => ({ locale: "ar", textScale: 1, brightness: "dark", reduceMotion: false, boldText: false, highContrast: false }),
  onSettings: () => 1,
  offSettings: () => {},
});
createRoot(() => {
  let a = 0, b = 0;
  const row = <Row style={{ width: 40 }}>
    <View ref={id => { a = id ?? 0; }} style={{ width: 10, height: 10 }} />
    <View ref={id => { b = id ?? 0; }} style={{ width: 10, height: 10 }} />
  </Row>;
  scene.append(root, row);
  checkDirection();
  function checkDirection() {
    assert(scene.rect(a).x > scene.rect(b).x, "initial Arabic theme port synchronizes native root direction");
  }
});
installThemePort({ settings: () => scene.settings(), onSettings: fn => scene.onSettings(fn), offSettings: id => scene.offSettings(id) });

// Ordinary component refs follow callback replacement and owner cleanup too.
createRoot((dispose) => {
  let mounts = 0, clears = 0;
  const first = (id: number | null) => id === null ? clears++ : mounts++;
  const second = (id: number | null) => id === null ? clears++ : mounts++;
  const [ref, setRef] = createSignal(first);
  const box = <View {...{ get ref() { return ref(); } }} />;
  scene.append(root, box);
  batch(() => setRef(() => second));
  dispose();
  assert(mounts === 2 && clears === 2, `View ref replacement and disposal match mounts: ${mounts}/${clears}`);
});
