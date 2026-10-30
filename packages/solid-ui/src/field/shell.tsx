import { createTrackedEffect, createSignal, onCleanup } from "solid-js";
import { Text, directionFor, host, onFrame, requestFrame, scrollIntoView, setRecognizers, setStroke, type NodeHandle } from "@impasto/solid-renderer";
import { colorBytes } from "../colors.ts";
import { coreHost } from "../host-ext.ts";
import { bindFocusRing } from "../focus.ts";
import { Role } from "../semantics.ts";
import { useTheme } from "../use-theme.ts";
import { CaretLayer } from "./caret.tsx";
import { createFieldController, type FieldController, type InputMask } from "./controller.ts";
import { enterInsertsNewline, imeConfig, type ActionName, type KeyboardName } from "./keyboard.ts";

const LINE = 22;

export type FieldProps = {
  value?: string;
  placeholder?: string;
  onChange?: (value: string) => void;
  onSubmit?: (value: string) => void;
  multiline?: boolean;
  keyboard?: KeyboardName;
  action?: ActionName;
  maxLines?: number;
  maxLength?: number;
  disabled?: boolean;
  readOnly?: boolean;
  error?: string;
  validate?: (value: string) => string | null | undefined;
  label?: string;
  rows?: number;
  mask?: InputMask;
  transform?: (value: string) => string;
  controller?: (controller: FieldController) => void;
};

function utf16Len(value: string): number {
  let count = 0;
  for (const char of value) {
    count += (char.codePointAt(0) ?? 0) > 0xffff ? 2 : 1;
  }
  return count;
}

function findText(id: number): number {
  let child = host.firstChild(id);
  while (child !== 0) {
    if (host.isText(child)) {
      return child;
    }
    const nested = findText(child);
    if (nested !== 0) {
      return nested;
    }
    child = host.nextSibling(child);
  }
  return 0;
}

function maskPassword(raw: string): string {
  return "\u2022".repeat(coreHost.graphemeCount(raw));
}

/**
 * Decorated field chrome: padding, hint, error, and the single-line scroller.
 * `follow` asks the host to scroll the caret into view.
 */
export function FieldShell(props: FieldProps): NodeHandle {
  const theme = useTheme();
  const [focused, setFocused] = createSignal(false);
  const [blink, setBlink] = createSignal(true);
  const [caretX, setCaretX] = createSignal(0);
  const [caretY, setCaretY] = createSignal(0);
  const [caretH, setCaretH] = createSignal(LINE);
  const [rects, setRects] = createSignal<{ x: number; y: number; w: number; h: number }[]>([]);
  let node = 0;
  let box = 0;

  const controller = createFieldController({
    textEdit: (op, extra) => {
      const id = leaf();
      if (id === 0) {
        return { text: "", start: 0, end: 0 };
      }
      return host.textEdit(id, op, extra);
    },
    onChange: (value) => props.onChange?.(value),
    validate: props.validate,
    maxLength: () => props.maxLength,
    focus: () => {
      if (node !== 0) {
        host.focus(node);
      }
    },
    onVertical: (direction) => {
      host.focusStep(direction);
    },
    allowChange: () => !frozen(),
  });
  props.controller?.(controller);

  const password = () => props.keyboard === "password";
  const multiline = () => !password() && !!props.multiline;
  const blocked = () => !!props.disabled;
  const frozen = () => blocked() || !!props.readOnly;
  const hint = () => controller.text().length === 0 && controller.composing() == null;
  const shownError = () => {
    const override = props.error;
    if (override !== undefined && override.length > 0) {
      return override;
    }
    return controller.error();
  };

  const shown = () => {
    if (hint()) {
      return props.placeholder ?? "";
    }
    if (password()) {
      return maskPassword(controller.text());
    }
    return controller.text();
  };

  function leaf(): number {
    return node === 0 ? 0 : findText(node);
  }

  function clusterIndex(raw: string, utf16: number): number {
    if (!password() || raw.length === 0) {
      return utf16;
    }
    return coreHost.graphemeIndex(raw, utf16);
  }

  function utf16OfCluster(raw: string, index: number): number {
    if (!password() || raw.length === 0) {
      return index;
    }
    return coreHost.graphemeUnit(raw, index);
  }

  function paint(): void {
    const id = leaf();
    if (id === 0) {
      return;
    }
    host.setText(id, shown());
  }

  function role(): number {
    if (props.action === "search") {
      return Role.SearchField;
    }
    if (multiline()) {
      return Role.TextArea;
    }
    return Role.TextField;
  }

  function writeSemantics(): void {
    if (node === 0) {
      return;
    }
    const editable = !blocked() && !props.readOnly ? 1 : 0;
    const masked = password();
    host.setFocusable(node, !blocked());
    host.setSemantics(
      node,
      role(),
      props.label,
      masked ? undefined : controller.text(),
      undefined,
      masked ? undefined : controller.selection().start,
      masked ? undefined : controller.selection().end,
      blocked() ? 1 : 0,
      editable,
      props.readOnly ? 1 : 0,
      shownError() || undefined,
      multiline() ? { order: props.rows } : undefined,
    );
  }

  function publish(show: boolean): void {
    if (!show || blocked()) {
      host.setImeState(null);
      return;
    }
    const full = controller.text();
    const caret = controller.selection().end;
    const anchor = controller.selection().start;
    const from = controller.composeStart();
    const to = controller.composeEnd();
    const composingStart = from >= 0 && to > from ? from : -1;
    const composingEnd = from >= 0 && to > from ? to : -1;
    const id = leaf();
    let x = caretX();
    let y = caretY();
    let h = caretH();
    if (id !== 0) {
      const textBox = host.rect(id);
      x = textBox.x + caretX();
      y = textBox.y + caretY();
    }
    const config = imeConfig(props.keyboard, props.action, multiline());
    host.setImeConfig(config.keyboard, config.action);
    host.setImeState(full, anchor, caret, composingStart, composingEnd, x, y, 1, h || LINE);
  }

  /** Scroll the caret's text node into the nearest scroll ancestor. */
  function follow(): void {
    const id = leaf();
    if (id === 0) {
      return;
    }
    scrollIntoView(id, 0);
  }

  bindFocusRing(() => box, focused);

  onCleanup(
    onFrame((targetMs) => {
      if (!focused()) {
        return;
      }
      const id = leaf();
      const raw = controller.text();
      if (id !== 0 && raw.length > 0) {
        const hit = host.caretAtUnit(id, clusterIndex(raw, controller.selection().end));
        setCaretX(hit.x);
        setCaretY(hit.y);
        setCaretH(hit.h || LINE);
      }
      if (host.reduceMotion()) {
        setBlink(true);
        return;
      }
      setBlink(Math.floor(targetMs / 500) % 2 === 0);
      requestFrame();
    }),
  );

  createTrackedEffect(() => {
    const unit = controller.selection().end;
    const value = controller.text();
    const anchor = controller.selection().start;
    shown();
    const id = leaf();
    if (id === 0 || value.length === 0) {
      setCaretX(0);
      setCaretY(0);
      setCaretH(LINE);
      setRects([]);
      paint();
      return;
    }
    const display = clusterIndex(value, unit);
    const hit = host.caretAtUnit(id, display);
    setCaretX(hit.x);
    setCaretY(hit.y);
    setCaretH(hit.h || LINE);
    const from = Math.min(anchor, unit);
    const to = Math.max(anchor, unit);
    if (from === to) {
      setRects([]);
    } else if (password()) {
      setRects(host.selectionRects(id, clusterIndex(value, from), clusterIndex(value, to)));
    } else {
      setRects(host.selectionRects(id, from, to));
    }
    paint();
    follow();
  });

  createTrackedEffect(() => {
    const value = props.value;
    if (value !== undefined) {
      controller.load(value);
      paint();
    }
  });

  createTrackedEffect(() => {
    if (props.mask) {
      controller.setMask(props.mask);
    }
    if (props.transform) {
      controller.setTransform(props.transform);
    }
  });

  createTrackedEffect(() => {
    writeSemantics();
  });

  createTrackedEffect(() => {
    const color = colorBytes(border());
    if (box !== 0) {
      setStroke(box, 1, color);
    }
  });

  const minHeight = (): number => {
    if (props.rows !== undefined && props.rows > 0) {
      return props.rows * LINE;
    }
    return 48;
  };

  const border = (): string => {
    if (shownError()) {
      return theme().color.danger;
    }
    if (focused()) {
      return theme().color.focusRing;
    }
    return theme().color.outline;
  };

  return (
    <column gap={theme().space.s} w="100%">
      <pointer
        ref={(handle) => {
          node = handle.id;
          host.setDragClaim(handle.id, multiline() ? "none" : "horizontal");
          if (multiline()) {
            setRecognizers(handle.id, { longPress: true, drag: "none" });
          }
          host.setFocusable(handle.id, !blocked());
          writeSemantics();
        }}
        w="100%"
        min={{ h: minHeight() }}
          onFocus={() => {
            if (blocked()) {
              return;
            }
            setFocused(true);
            setBlink(true);
            publish(true);
            follow();
            requestFrame();
          }}
          onBlur={() => {
            setFocused(false);
            publish(false);
          }}
          onPress={() => {
            if (blocked()) {
              return;
            }
            if (node !== 0) {
              host.focus(node);
            }
            const id = leaf();
            if (id === 0) {
              return;
            }
            const point = host.pointerPosition();
            const textBox = host.rect(id);
            const inside =
              point.x >= textBox.x &&
              point.x <= textBox.x + Math.max(textBox.w, 1) &&
              point.y >= textBox.y &&
              point.y <= textBox.y + Math.max(textBox.h, 1);
            if (inside) {
              const caretHit = host.caretAt(id, point.x, point.y);
              controller.caretAt(utf16OfCluster(controller.text(), caretHit.unit));
            } else if (controller.text().length > 0) {
              controller.caretAt(utf16Len(controller.text()));
            }
            publish(true);
          }}
          onDragBy={() => {
            if (blocked()) {
              return;
            }
            const id = leaf();
            if (id === 0) {
              return;
            }
            const point = host.pointerPosition();
            const caretHit = host.caretAt(id, point.x, point.y);
            controller.extendCaret(utf16OfCluster(controller.text(), caretHit.unit));
            publish(true);
          }}
          onKey={(key) => {
            if (blocked()) {
              return;
            }
            controller.handleKey(
              key,
              multiline(),
              password(),
              !enterInsertsNewline(multiline(), props.action),
              () => props.onSubmit?.(controller.text()),
            );
            publish(true);
            paint();
          }}
          onText={(value) => {
            if (frozen()) {
              return;
            }
            controller.onText(value);
            publish(true);
            paint();
          }}
          onTextReplaced={(value, anchor, caret, composeFrom, composeTo) => {
            if (frozen()) {
              return;
            }
            if (typeof anchor === "number" && typeof caret === "number") {
              controller.replace(
                value,
                anchor,
                caret,
                typeof composeFrom === "number" ? composeFrom : -1,
                typeof composeTo === "number" ? composeTo : -1,
              );
            } else {
              controller.onTextReplaced(value);
            }
            publish(focused());
            paint();
          }}
          onComposing={(value) => {
            if (frozen()) {
              return;
            }
            controller.onComposing(value);
            publish(true);
            paint();
          }}
      >
        <decorated
          ref={(handle) => {
            box = handle.id;
            setStroke(handle.id, 1, colorBytes(border()));
          }}
          color={theme().color.surface}
          radius={theme().shape.l}
          w="100%"
          min={{ h: minHeight() }}
        >
          <opacity alpha={blocked() ? 102 : 255}>
            <padding edges={theme().space.l} w="100%">
              <scroll axis={multiline() ? "vertical" : "horizontal"}>
                  <padding right={2}>
                    <stack>
                      <Text
                        fontSize={theme().typography.body.size}
                        color={hint() ? theme().color.textMuted : theme().color.text}
                        weight={theme().typography.body.weight}
                        direction={directionFor(theme().locale)}
                        softWrap={multiline()}
                        overflow={multiline() ? "clip" : "visible"}
                        maxLines={multiline() ? props.maxLines : undefined}
                      >
                        {shown()}
                      </Text>
                      <CaretLayer
                        rects={rects}
                        caretX={caretX}
                        caretY={caretY}
                        caretH={caretH}
                        showCaret={() => focused() && blink() && controller.selection().start === controller.selection().end}
                        accent={() => theme().color.accent}
                        transparent={() => theme().color.transparent}
                      />
                    </stack>
                  </padding>
                </scroll>
            </padding>
          </opacity>
        </decorated>
      </pointer>
      {shownError() ? (
        <Text
          fontSize={theme().typography.body.size}
          color={theme().color.danger}
          weight={theme().typography.body.weight}
          direction={directionFor(theme().locale)}
        >
          {shownError()}
        </Text>
      ) : null}
    </column>
  );
}
