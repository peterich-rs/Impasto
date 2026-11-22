import { createRoot, createSignal } from "solid-js";
import { boot, scene } from "@impasto/solid-renderer";
import { Button, Column, Pressable } from "@impasto/solid-ui";

createRoot(() => {
  function control(button: boolean) {
    const name = button ? "button" : "pressable";
    const [count, setCount] = createSignal(0);
    const log = (event: string) => scene.log("info", `${name}:${event}`);
    const events = {
      get onPress() { return count() === 0 ? () => log("old-press") : () => log("new-press"); },
      onRelease: () => log("release"),
      onClick: () => { setCount(count() + 1); log("activate"); },
      onText: (value: string) => log(`text:${value}`),
      onKey: (value: string) => log(`key:${value}`),
      onFocus: () => log("focus"),
      onBlur: () => log("blur"),
      onHover: () => log("hover"),
      onLeave: () => log("leave"),
      onWheel: () => log("wheel"),
      onComposing: () => log("composing"),
      onTextReplaced: () => log("replace"),
    };
    return button
      ? <Button {...events} label={name} disabled={count() >= 4} style={{ width: 100, height: 50 }} />
      : <Pressable {...events} accessibilityLabel={name} disabled={count() >= 4}
          style={{ width: 100, height: 50 }} />;
  }
  boot(() => <Column style={{ width: 240, height: 200 }}>
    {control(true)}{control(false)}
  </Column>);
});
