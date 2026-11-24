import { createTrackedEffect, createSignal, onCleanup } from "solid-js";
import {
  Text,
  bindNodeRef,
  bindStyle,
  scene,
  onFrame,
  requestFrame,
  scrollIntoView,
  setRecognizers,
  type NodeHandle,
  type StyleInput,
} from "@impasto/solid-renderer";
import { StyledText } from "../styled-text.ts";
import { colorBytes } from "../colors.ts";
import { sceneApi } from "../scene-ext.ts";
import { bindFocusRing } from "../focus.ts";
import { Role } from "../semantics.ts";
import { useTheme } from "../use-theme.ts";
import { CaretLayer } from "./caret.tsx";
import { createFieldController } from "./controller.ts";
import type { FieldProps } from "../props.ts";
import { enterInsertsNewline, imeConfig } from "./keyboard.ts";
const LINE = 22;
function utf16Len(value: string): number {
  let count = 0;
  for (const char of value) {
    count += (char.codePointAt(0) ?? 0) > 0xffff ? 2 : 1;
  }
  return count;
}
function findText(id: number): number {
  let child = scene.firstChild(id);
  while (child !== 0) {
    if (scene.isText(child)) {
      return child;
    }
    const nested = findText(child);
    if (nested !== 0) {
      return nested;
    }
    child = scene.nextSibling(child);
  }
  return 0;
}
function maskPassword(raw: string): string {
  return "\u2022".repeat(sceneApi.graphemeCount(raw));
}

/**
 * Decorated field chrome: padding, hint, error, and the single-line scroller.
 * `follow` asks the scene to scroll the caret into view.
 */
export function FieldShell(props: FieldProps): NodeHandle {
  const theme = useTheme();
  const [focused, setFocused] = createSignal(false);
  const [blink, setBlink] = createSignal(true);
  const [caretX, setCaretX] = createSignal(0);
  const [caretY, setCaretY] = createSignal(0);
  const [caretH, setCaretH] = createSignal(LINE);
  const [rects, setRects] = createSignal<
    {
      x: number;
      y: number;
      w: number;
      h: number;
    }[]
  >([]);
  let node = 0;
  let box = 0;
  const controller = createFieldController({
    textEdit: (op, extra) => {
      const id = leaf();
      if (id === 0) {
        return {
          text: "",
          start: 0,
          end: 0,
        };
      }
      return scene.textEdit(id, op, extra);
    },
    textCommand: (name, extra, recordUndo) => {
      const id = leaf();
      if (id === 0) {
        return {
          text: extra,
          start: 0,
          end: 0,
        };
      }
      return scene.textCommand(id, name, extra, recordUndo);
    },
    onChange: (value) => props.onChange?.(value),
    validate: props.validate,
    maxLength: () => props.maxLength,
    focus: () => {
      if (node !== 0) {
        scene.focus(node);
      }
    },
    onVertical: (direction) => {
      scene.focusStep(direction);
    },
    allowChange: () => !frozen(),
  });
  props.controller?.(controller);
  createTrackedEffect(() => {
    const id = leaf();
    if (id !== 0) {
      scene.setTextLimit(id, props.maxLength ?? -1);
    }
  });
  const password = () => props.keyboard === "password";
  const multiline = () => !password() && !!props.multiline;
  const blocked = () => !!props.disabled;
  const frozen = () => blocked() || !!props.readOnly;
  const hint = () =>
    controller.text().length === 0 && controller.composing() == null;
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
    return sceneApi.graphemeIndex(raw, utf16);
  }
  function utf16OfCluster(raw: string, index: number): number {
    if (!password() || raw.length === 0) {
      return index;
    }
    return sceneApi.graphemeUnit(raw, index);
  }
  function paint(): void {
    const id = leaf();
    if (id === 0) {
      return;
    }
    scene.setText(id, shown());
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
    scene.setFocusable(node, !blocked());
    scene.setSemantics(
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
      {
        password: masked,
        order: multiline() ? props.rows : undefined,
        autofill: props.autofill && editable
          ? { hint: props.autofill, value: controller.text() }
          : undefined,
      },
    );
  }
  function publish(show: boolean): void {
    if (!show || blocked()) {
      scene.setImeState(null);
      return;
    }
    const full = controller.text();
    const caret = controller.selection().end;
    const anchor = controller.selection().start;
    const from = controller.composeStart();
    const to = controller.composeEnd();
    const composingStart = from >= 0 && to > from ? from : -1;
    const composingEnd = from >= 0 && to > from ? to : -1;
    const config = imeConfig(props.keyboard, props.action, multiline());
    scene.setImeConfig(config.keyboard, config.action);
    scene.setImeState(
      full,
      anchor,
      caret,
      composingStart,
      composingEnd,
      0,
      0,
      1,
      caretH() || LINE,
    );
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
  createTrackedEffect(() => {
    if (!focused()) return;
    onCleanup(onFrame((targetMs) => {
      const id = leaf();
      const raw = controller.text();
      if (id !== 0 && raw.length > 0) {
        const hit = scene.caretAtUnit(
          id,
          clusterIndex(raw, controller.selection().end),
        );
        setCaretX(hit.x);
        setCaretY(hit.y);
        setCaretH(hit.h || LINE);
      }
      if (scene.reduceMotion()) {
        setBlink(true);
        return;
      }
      setBlink(Math.floor(targetMs / 500) % 2 === 0);
      requestFrame();
    }));
  });
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
    const hit = scene.caretAtUnit(id, display);
    setCaretX(hit.x);
    setCaretY(hit.y);
    setCaretH(hit.h || LINE);
    const from = Math.min(anchor, unit);
    const to = Math.max(anchor, unit);
    if (from === to) {
      setRects([]);
    } else if (password()) {
      setRects(
        scene.selectionRects(
          id,
          clusterIndex(value, from),
          clusterIndex(value, to),
        ),
      );
    } else {
      setRects(scene.selectionRects(id, from, to));
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
      scene.patchStyle(box, {
        outlineWidth: 1,
        outlineColor: color,
      });
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
    <view
      ref={(handle) => {
        if (handle === null) return;
        bindStyle(handle, {
          defaults: {
            gap: theme().space.s,
            width: "100%",
          },
          get class() {
            return props.class;
          },
          get classList() {
            return props.classList;
          },
          get style() {
            return props.style;
          },
        });
        bindNodeRef(handle, () => props.ref);
      }}
    >
      {(props.label ?? "").length > 0 ? (
        <Text
          class={props.labelProps?.class}
          classList={props.labelProps?.classList}
          style={props.labelProps?.style}
        >
          {props.label}
        </Text>
      ) : null}
      <view
        ref={(handle) => {
          if (handle === null) return;
          node = handle;
          bindNodeRef(handle, () => props.inputRef);
          bindStyle(handle, {
            get defaults(): StyleInput {
              return {
                width: "100%",
                minHeight: minHeight(),
              };
            },
            get class() {
              return props.controlProps?.class;
            },
            get classList() {
              return props.controlProps?.classList;
            },
            get style() {
              return props.controlProps?.style;
            },
          });
          scene.setDragClaim(handle, multiline() ? "none" : "horizontal");
          if (multiline()) {
            setRecognizers(handle, {
              longPress: true,
              drag: "none",
            });
          }
          scene.setFocusable(handle, !blocked());
          writeSemantics();
        }}
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
            scene.focus(node);
          }
          const id = leaf();
          if (id === 0) {
            return;
          }
          const point = scene.pointerPosition();
          const textBox = scene.rect(id);
          const inside =
            point.x >= textBox.x &&
            point.x <= textBox.x + Math.max(textBox.w, 1) &&
            point.y >= textBox.y &&
            point.y <= textBox.y + Math.max(textBox.h, 1);
          if (inside) {
            const caretHit = scene.caretAt(id, point.x, point.y);
            controller.caretAt(
              utf16OfCluster(controller.text(), caretHit.unit),
            );
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
          const point = scene.pointerPosition();
          const caretHit = scene.caretAt(id, point.x, point.y);
          controller.extendCaret(
            utf16OfCluster(controller.text(), caretHit.unit),
          );
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
        <view
          ref={(handle) => {
            if (handle === null) return;
            box = handle;
            scene.patchStyle(handle, {
              outlineWidth: 1,
              outlineColor: colorBytes(border()),
            });
          }}
          style={{
            backgroundColor: theme().color.surface,
            borderRadius: theme().shape.l,
            width: "100%",
            minWidth: undefined,
            minHeight: minHeight(),
          }}
        >
          <stack
            style={{
              opacity: (blocked() ? 102 : 255) / 255,
            }}
          >
            <view
              style={{
                padding: theme().space.l,
                width: "100%",
              }}
            >
              <scroll axis={multiline() ? "vertical" : "horizontal"}>
                <view
                  style={{
                    paddingRight: 2,
                  }}
                >
                  <stack>
                    <StyledText
                      class={props.textProps?.class}
                      classList={props.textProps?.classList}
                      style={props.textProps?.style}
                      defaults={{
                        fontSize: theme().typography.body.size,
                        color: hint()
                          ? theme().color.textMuted
                          : theme().color.text,
                        fontWeight: theme().typography.body.weight,
                        softWrap: multiline(),
                        textOverflow: multiline() ? "clip" : "visible",
                        maxLines: multiline() ? props.maxLines : undefined,
                      }}
                    >
                      {shown()}
                    </StyledText>
                    <CaretLayer
                      rects={rects}
                      caretX={caretX}
                      caretY={caretY}
                      caretH={caretH}
                      showCaret={() =>
                        focused() &&
                        blink() &&
                        controller.selection().start ===
                          controller.selection().end
                      }
                      accent={() => theme().color.accent}
                      transparent={() => theme().color.transparent}
                    />
                  </stack>
                </view>
              </scroll>
            </view>
          </stack>
        </view>
      </view>
      {shownError() ? (
        <StyledText
          class={props.errorProps?.class}
          classList={props.errorProps?.classList}
          style={props.errorProps?.style}
          defaults={{
            fontSize: theme().typography.body.size,
            color: theme().color.danger,
            fontWeight: theme().typography.body.weight,
          }}
        >
          {shownError()}
        </StyledText>
      ) : null}
    </view>
  );
}
