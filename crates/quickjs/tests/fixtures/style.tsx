import { batch, createRoot, createSignal } from "solid-js";
import { scene, bindStyle, Text } from "@impasto/solid-renderer";
import { Button } from "@impasto/solid-ui";
const assert = (condition: unknown, message: string) => {
  if (!condition) throw new Error(message);
};
createRoot((dispose) => {
  const root = <view style={{ width: 240, height: 320 }} />;
  const box = (
    <view style={[{ width: 10 }, false, { width: 20, height: undefined }]} />
  );
  scene.append(root, box);
  assert(scene.rect(box).w === 20, "array snapshot and synchronous rect");
  scene.setStyle(box, { width: 30, height: 12 });
  try {
    scene.setStyle(box, { width: 99, backgroundColor: { token: "missing" } });
  } catch {}
  assert(scene.rect(box).w === 30, "invalid snapshot changed layout");
  scene.patchStyle(box, { height: 13, width: undefined });
  assert(scene.rect(box).w === 30, "undefined patch overwrote width");
  scene.clearStyle(box, ["width"]);
  assert(scene.rect(box).w === 20, "clear did not reveal inline width");
  const padded = (
    <view>
      <view style={{ width: 10, height: 10 }} />
    </view>
  );
  scene.append(root, padded);
  scene.setStyle(padded, { padding: 4, width: "max-content" });
  assert(scene.rect(padded).w === 18, "padding snapshot");
  scene.clearStyle(padded, ["padding"]);
  assert(
    scene.rect(padded).w === 10,
    "clear shorthand did not remove its longhands",
  );
  const grid = (
    <view
      style={{
        display: "grid",
        gridTemplateColumns: [
          { fr: 1 },
          { min: { points: 0 }, max: { fr: 1 } },
        ],
      }}
    />
  );
  scene.append(root, grid);
  const effects = (
    <view
      style={{
        width: 12,
        height: 12,
        padding: "10%",
        marginLeft: "auto",
        backgroundColor: { token: "accent", opacity: 0.5 },
        blur: 1,
        backdrop: 2,
        blend: "multiply",
        shadow: { elevation: 2, color: "#00000080" },
        boxShadow: [{ offset: [1, 2], blurSigma: 2, color: "#00000080" }],
        gradient: {
          kind: "linear",
          from: [0, 0],
          to: [1, 1],
          stops: [
            { offset: 0, color: "#112233" },
            { offset: 1, color: "#ffffff" },
          ],
        },
      }}
    />
  );
  scene.append(root, effects);
  const utility = <view class="blur-sm shadow-md bg-accent/50" />;
  scene.append(root, utility);
  const [width, setWidth] = createSignal(11);
  const owned = <view />;
  scene.append(root, owned);
  let stop = () => {};
  createRoot((done) => {
    stop = done;
    bindStyle(owned, {
      get style() {
        return { width: width() };
      },
    });
  });
  assert(scene.rect(owned).w === 11, "initial binding");
  batch(() => setWidth(22));
  assert(scene.rect(owned).w === 22, "reactive binding");
  const independent = scene.createStyleSource(owned, "inline");
  scene.setStyleSource(independent, { height: 9 });
  stop();
  assert(scene.rect(owned).h === 9, "cleanup removed another source");
  scene.removeStyleSource(independent);
  const classes = <view class="w-4" />;
  scene.append(root, classes);
  assert(scene.rect(classes).w === 16, "class resolution");
  scene.setProp(classes, "class", "");
  assert(scene.rect(classes).w !== 16, "empty class was ignored");
  scene.setProp(classes, "classList", { "w-5": true });
  assert(scene.rect(classes).w === 20, "classList snapshot");
  try {
    scene.setProp(classes, "classList", {
      get "w-6"() {
        throw new Error("class getter");
      },
    });
  } catch {}
  assert(scene.rect(classes).w === 20, "throwing classList changed its source");
  scene.setProp(classes, "classList", {});
  assert(scene.rect(classes).w !== 20, "empty classList was ignored");
  const label = (
    <Text style={{ fontSize: 18, color: "#112233" }}>样式回归</Text>
  );
  scene.append(root, label);
  const button = (
    <Button
      label="原生按钮"
      class="w-24"
      style={{ width: 120 }}
      onClick={() => {}}
    />
  );
  scene.append(root, button);
  assert(scene.rect(button).h > 0, "single-view button layout");
  assert(
    scene.rect(button).w === 120,
    "button inline style overrides class and defaults",
  );
  const wideRow = <view style={{ width: 240, flexDirection: "row" }} />;
  const wideA = (
    <Button
      style={{ width: "100%", flexShrink: 1 }}
      label="A"
      onClick={() => {}}
    />
  );
  const wideB = (
    <Button
      style={{ width: "100%", flexShrink: 1 }}
      label="B"
      onClick={() => {}}
    />
  );
  scene.append(root, wideRow);
  scene.append(wideRow, wideA);
  scene.append(wideRow, wideB);
  assert(
    scene.rect(wideA).w === 120 && scene.rect(wideB).w === 120,
    "wide buttons must share a row without implicit parent caps",
  );
  let blurred = 0;
  const focusable = (
    <view
      onBlur={() => {
        blurred++;
      }}
      style={{ width: 10, height: 10 }}
    />
  );
  scene.append(root, focusable);
  scene.setFocusable(focusable, true);
  scene.focus(focusable);
  const hiddenSource = scene.createStyleSource(focusable, "inline");
  scene.setStyleSource(hiddenSource, { display: "none" });
  assert(blurred === 1, "owned display:none did not dispatch blur");
  scene.removeStyleSource(hiddenSource);
  // The whole input is copied before Rust borrows Tree. Reentrant getters
  // may perform independent writes, but a throwing snapshot is never accepted.
  try {
    scene.setStyle(box, {
      get width() {
        scene.patchStyle(box, { height: 17 });
        throw new Error("getter");
      },
    });
  } catch {}
  assert(
    scene.rect(box).w === 20 && scene.rect(box).h === 17,
    "getter exception was not atomic",
  );
  dispose();
});
