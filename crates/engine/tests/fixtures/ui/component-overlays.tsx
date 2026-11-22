import { createRoot, createSignal } from "solid-js";
import { boot, onFrame, requestFrame, scene } from "@impasto/solid-renderer";
import { Column, Dialog, Menu, Sheet, Text, Toast, Tooltip, View } from "@impasto/solid-ui";

const check = (condition: unknown, message: string) => { if (!condition) throw new Error(message); };
let stage = 0;
let dispose = () => {};
let toggle = (_open: boolean) => {};
const refs = new Map<string, number>();
const mounts = new Map<string, number>();
const clears = new Map<string, number>();
const ref = (name: string) => (node: number | null) => {
  if (node === null) { refs.delete(name); clears.set(name, (clears.get(name) ?? 0) + 1); }
  else { refs.set(name, node); mounts.set(name, (mounts.get(name) ?? 0) + 1); }
};
createRoot((done) => {
  dispose = done;
  const [open, setOpen] = createSignal(false);
  toggle = setOpen;
  const [size, setSize] = createSignal(80);
  const [textSize, setTextSize] = createSignal(24);
  let anchor = 0;
  boot(() => <Column style={{ width: 240, height: 400 }}>
    <View ref={id => { anchor = id ?? 0; }} style={{ width: 20, height: 20 }} />
    <Dialog ref={ref("dialog")} open={open()} onClose={() => {}} title="dialog-title"
      class="gap-4" style={{ width: size() }} contentProps={{ class: "gap-4" }}
      titleProps={{ class: "text-4xl" }}><Text>dialog-child</Text></Dialog>
    <Sheet ref={ref("sheet")} open={open()} onClose={() => {}} title="sheet-title"
      style={{ width: size() }} titleProps={{ class: "text-4xl" }}><Text>sheet-child</Text></Sheet>
    <Menu ref={ref("menu")} open={open()} anchor={anchor} onClose={() => {}}
      style={{ width: size() }} contentProps={{ class: "gap-4" }}><Text>menu-child</Text></Menu>
    <Toast ref={ref("toast")} open={open()} onClose={() => {}} text="toast-label" durationMs={100000}
      style={{ width: size() }} textProps={{ style: [{ fontSize: textSize() }] }} />
    <Tooltip ref={ref("tooltip")} open={open()} anchor={anchor} text="tooltip-label"
      style={{ width: size() }} textProps={{ class: "text-4xl" }} />
  </Column>);
  onFrame(() => {
    stage++;
    if (stage === 1) setOpen(true);
    if (stage === 2) {
      for (const [name, node] of refs) check(scene.rect(node).w === 80, `${name} initial width`);
      check(refs.size === 5, "all delayed overlay panels attach");
      setSize(120); setTextSize(30);
    }
    if (stage === 3) {
      for (const [name, node] of refs) check(scene.rect(node).w === 120, `${name} reactive width`);
      // Rust inspects typography in this frame before closing the panels.
    }
    if (stage === 4) setOpen(false);
    if (stage === 5) {
      check(refs.has("dialog") && refs.has("sheet"), "modal refs survive their exit animation");
    }
    if (stage < 6) requestFrame();
  });
});

// The verifier survives disposal of the component owner.
const stop = onFrame((ms) => {
  if (ms === 2000 || ms === 4000) {
    const expected = ms === 2000 ? 1 : 2;
    check(refs.size === 0, "all panels close after their exit animations");
    for (const name of ["dialog", "sheet", "menu", "toast", "tooltip"]) {
      check(mounts.get(name) === expected && clears.get(name) === expected, `${name} paired panel refs`);
    }
    if (ms === 2000) toggle(true);
    else {
      dispose();
      for (const count of clears.values()) check(count === 2, "owner disposal does not repeat null");
      scene.log("info", "overlays:complete");
      stop();
    }
  }
  if (ms === 2016) {
    check(refs.size === 5, "all panels reopen with new node refs");
    toggle(false);
  }
});
