import { createRenderEffect, createSignal, onCleanup } from "solid-js";
import { Text, host, onFrame, requestFrame, type NodeHandle } from "@uifw/solid-renderer";
import { coreHost } from "./host-ext.ts";
import { directionFor } from "./i18n.ts";
import { Role } from "./semantics.ts";
import { useTheme } from "./use-theme.ts";

const LINE = 22;

const EDIT_LOAD = 0;
const EDIT_LEFT = 1;
const EDIT_RIGHT = 2;
const EDIT_BACKSPACE = 3;
const EDIT_INSERT = 4;
const EDIT_COMPOSING = 5;
const EDIT_SELECT_ALL = 9;
const EDIT_COPY = 10;
const EDIT_CUT = 11;
const EDIT_CARET = 12;
const EDIT_FORWARD_DELETE = 13;
const EDIT_NEWLINE = 14;
const EDIT_WORD_LEFT = 15;
const EDIT_WORD_RIGHT = 16;
const EDIT_LINE_START = 17;
const EDIT_LINE_END = 18;
const EDIT_EXTEND_LEFT = 19;
const EDIT_EXTEND_RIGHT = 20;
const EDIT_EXTEND_WORD_LEFT = 21;
const EDIT_EXTEND_WORD_RIGHT = 22;
const EDIT_EXTEND_LINE_START = 23;
const EDIT_EXTEND_LINE_END = 24;
const EDIT_UP = 25;
const EDIT_DOWN = 26;
const EDIT_EXTEND_UP = 27;
const EDIT_EXTEND_DOWN = 28;
const EDIT_EXTEND_CARET = 29;
const EDIT_DELETE_WORD = 30;
const EDIT_DELETE_WORD_FORWARD = 31;
const EDIT_DELETE_TO_LINE_START = 32;
const EDIT_DELETE_TO_LINE_END = 33;
const EDIT_DOCUMENT_START = 34;
const EDIT_DOCUMENT_END = 35;
const EDIT_EXTEND_DOCUMENT_START = 36;
const EDIT_EXTEND_DOCUMENT_END = 37;

const KEYBOARD: Record<string, number> = {
  text: 0,
  multiline: 1,
  number: 2,
  phone: 3,
  email: 4,
  url: 5,
  name: 6,
  password: 7,
};

const ACTION: Record<string, number> = {
  none: 0,
  done: 1,
  next: 2,
  search: 3,
  go: 4,
  send: 5,
  previous: 6,
  newline: 7,
};

const SUBMIT_ACTIONS = new Set(["done", "search", "go", "send", "next"]);

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

/** Scroll the nearest scroll ancestor so `id`'s layout box sits inside the viewport. */
function reveal(id: number): void {
  const field = host.rect(id);
  let parent = host.parent(id);
  while (parent !== 0) {
    let offset: { x: number; y: number };
    try {
      offset = host.scrollOffset(parent);
    } catch {
      parent = host.parent(parent);
      continue;
    }
    const view = host.rect(parent);
    let y = offset.y;
    const paintedBottom = field.y - y + field.h;
    if (paintedBottom > view.y + view.h) {
      y += paintedBottom - (view.y + view.h);
    }
    if (field.y - y < view.y) {
      y -= view.y - (field.y - y);
    }
    if (y < 0) {
      y = 0;
    }
    if (y !== offset.y) {
      host.setScrollOffset(parent, offset.x, y);
    }
    parent = host.parent(parent);
  }
}

function maskPassword(raw: string): string {
  return "\u2022".repeat(coreHost.graphemeCount(raw));
}

/**
 * A text field. The string, selection, and composing range live in `core`'s
 * text editor. This component binds that value to a `Text` node and the
 * platform keyboard.
 *
 * A single-line field does not wrap; its own scroll keeps the caret in view.
 * A multiline field breaks on width and on inserted newlines. Up and down move
 * the caret there, and move focus in a single-line field.
 */
export function Field(props: {
  value?: string;
  placeholder?: string;
  onChange?: (value: string) => void;
  onSubmit?: (value: string) => void;
  multiline?: boolean;
  keyboard?: "text" | "number" | "phone" | "email" | "url" | "name" | "password";
  action?: "none" | "done" | "next" | "search" | "go" | "send" | "previous" | "newline";
  maxLines?: number;
  maxLength?: number;
  disabled?: boolean;
  readOnly?: boolean;
  error?: string;
  validate?: (value: string) => string | null | undefined;
  label?: string;
}): NodeHandle {
  const theme = useTheme();
  const [text, setText] = createSignal("");
  const [composing, setComposing] = createSignal<string | null>(null);
  const [focused, setFocused] = createSignal(false);
  const [blink, setBlink] = createSignal(true);
  const [caretX, setCaretX] = createSignal(0);
  const [caretY, setCaretY] = createSignal(0);
  const [caretH, setCaretH] = createSignal(LINE);
  const [base, setBase] = createSignal(0);
  const [extent, setExtent] = createSignal(0);
  const [rects, setRects] = createSignal<{ x: number; y: number; w: number; h: number }[]>([]);
  const [validError, setValidError] = createSignal("");
  let node = 0;
  let view = 0;

  const password = () => props.keyboard === "password";
  const multiline = () => !password() && !!props.multiline;
  const blocked = () => !!props.disabled;
  const frozen = () => blocked() || !!props.readOnly;
  const hint = () => text().length === 0 && composing() == null;
  const shownError = () => {
    const override = props.error;
    if (override !== undefined && override.length > 0) {
      return override;
    }
    return validError();
  };

  const shown = () => {
    if (hint()) {
      return props.placeholder ?? "";
    }
    if (password()) {
      return maskPassword(text());
    }
    return text();
  };

  function leaf(): number {
    return node === 0 ? 0 : findText(node);
  }

  /** Bullet-string unit for a real UTF-16 offset. One bullet is one cluster. */
  function clusterIndex(raw: string, utf16: number): number {
    if (!password() || raw.length === 0) {
      return utf16;
    }
    return coreHost.graphemeIndex(raw, utf16);
  }

  /** Real UTF-16 start for a cluster index measured on the bullet string. */
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

  function apply(
    op: number,
    extra = "",
    kind: "load" | "motion" | "change" | "compose",
  ): void {
    if (blocked()) {
      return;
    }
    if (frozen() && (kind === "change" || kind === "compose")) {
      return;
    }
    const id = leaf();
    if (id === 0) {
      return;
    }
    const before = text();
    const next = host.textEdit(id, op, extra);
    if (
      kind !== "load" &&
      props.maxLength !== undefined &&
      utf16Len(next.text) > props.maxLength
    ) {
      const reverted = host.textEdit(id, EDIT_LOAD, before);
      setText(reverted.text);
      setBase(reverted.start);
      setExtent(reverted.end);
      paint();
      return;
    }
    setText(next.text);
    setBase(next.start);
    setExtent(next.end);
    paint();
    if (kind === "change" && next.text !== before) {
      props.onChange?.(next.text);
      if (props.validate) {
        setValidError(props.validate(next.text) ?? "");
      } else {
        setValidError("");
      }
    }
  }

  function keyboardCode(): number {
    if (props.keyboard && props.keyboard !== "text") {
      return KEYBOARD[props.keyboard] ?? 0;
    }
    return multiline() ? 1 : 0;
  }

  function actionCode(): number {
    if (props.action) {
      return ACTION[props.action] ?? 0;
    }
    return multiline() ? 7 : 1;
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
      Role.TextField,
      props.label,
      masked ? undefined : text(),
      undefined,
      masked ? undefined : base(),
      masked ? undefined : extent(),
      blocked() ? 1 : 0,
      editable,
      props.readOnly ? 1 : 0,
      shownError() || undefined,
    );
  }

  function publish(show: boolean): void {
    if (!show || blocked()) {
      host.setImeState(null);
      return;
    }
    const full = text();
    const extra = composing() ?? "";
    const caret = extent();
    const anchor = base();
    const composingStart = extra.length === 0 ? -1 : Math.max(0, utf16Len(full) - utf16Len(extra));
    const composingEnd = extra.length === 0 ? -1 : utf16Len(full);
    const id = leaf();
    let x = caretX();
    let y = caretY();
    let h = caretH();
    if (id !== 0 && view !== 0) {
      const textBox = host.rect(id);
      const offset = host.scrollOffset(view);
      x = textBox.x + caretX() - offset.x;
      y = textBox.y + caretY() - offset.y;
    }
    host.setImeConfig(keyboardCode(), actionCode());
    host.setImeState(full, anchor, caret, composingStart, composingEnd, x, y, 1, h || LINE);
  }

  function follow(): void {
    if (view === 0) {
      return;
    }
    const id = leaf();
    if (id === 0) {
      return;
    }
    const box = host.rect(view);
    const textBox = host.rect(id);
    const offset = host.scrollOffset(view);
    let x = offset.x;
    let y = offset.y;
    const left = textBox.x + caretX() - x;
    const top = textBox.y + caretY() - y;
    const bottom = top + caretH();
    if (left + 1 > box.x + box.w) {
      x += left + 1 - (box.x + box.w);
    }
    if (left < box.x) {
      x -= box.x - left;
    }
    if (bottom > box.y + box.h) {
      y += bottom - (box.y + box.h);
    }
    if (top < box.y) {
      y -= box.y - top;
    }
    if (x < 0) {
      x = 0;
    }
    if (y < 0) {
      y = 0;
    }
    if (x !== offset.x || y !== offset.y) {
      host.setScrollOffset(view, x, y);
    }
  }

  onCleanup(
    onFrame((targetMs) => {
      if (!focused()) {
        return;
      }
      const id = leaf();
      const raw = text();
      if (id !== 0 && raw.length > 0) {
        const hit = host.caretAtUnit(id, clusterIndex(raw, extent()));
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

  createRenderEffect(
    () => [extent(), text(), base(), shown()] as const,
    ([unit, value, anchor]) => {
      const id = leaf();
      if (id === 0 || value.length === 0) {
        setCaretX(0);
        setCaretY(0);
        setCaretH(LINE);
        setRects([]);
        return;
      }
      const display = clusterIndex(value, unit);
      const hit = host.caretAtUnit(id, display);
      setCaretX(hit.x);
      setCaretY(hit.y);
      setCaretH(hit.h || LINE);
      const start = Math.min(anchor, unit);
      const end = Math.max(anchor, unit);
      if (start === end) {
        setRects([]);
      } else if (password()) {
        setRects(host.selectionRects(id, clusterIndex(value, start), clusterIndex(value, end)));
      } else {
        setRects(host.selectionRects(id, start, end));
      }
      follow();
    },
  );

  createRenderEffect(
    () => props.value,
    (value) => {
      if (value !== undefined && value !== text()) {
        apply(EDIT_LOAD, value, "load");
      }
    },
  );

  createRenderEffect(() => {
    writeSemantics();
  });

  function move(collapsed: number, extend: number, shift: boolean): void {
    apply(shift ? extend : collapsed, "", "motion");
    publish(true);
  }

  function submitEnter(): boolean {
    return !multiline() || SUBMIT_ACTIONS.has(props.action ?? "");
  }

  return (
    <column gap={theme().space.s}>
      <opacity alpha={blocked() ? 102 : 255}>
        <pointer
          ref={(handle) => {
            node = handle.id;
            host.setDragClaim(handle.id, "horizontal");
            host.setFocusable(handle.id, !blocked());
            writeSemantics();
          }}
          min={{ w: 280, h: 48 }}
          onFocus={() => {
            if (blocked()) {
              return;
            }
            setFocused(true);
            setBlink(true);
            publish(true);
            reveal(node);
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
            const id = leaf();
            if (id === 0) {
              return;
            }
            const point = host.pointerPosition();
            const caretHit = host.caretAt(id, point.x, point.y);
            apply(EDIT_CARET, String(utf16OfCluster(text(), caretHit.unit)), "motion");
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
            apply(EDIT_EXTEND_CARET, String(utf16OfCluster(text(), caretHit.unit)), "motion");
            publish(true);
          }}
          onKey={(key) => {
            if (blocked()) {
              return;
            }
            const shift = key.startsWith("Shift+");
            const name = shift ? key.slice("Shift+".length) : key;
            if (name === "Left") {
              move(EDIT_LEFT, EDIT_EXTEND_LEFT, shift);
              return;
            }
            if (name === "Right") {
              move(EDIT_RIGHT, EDIT_EXTEND_RIGHT, shift);
              return;
            }
            if (name === "Up") {
              if (multiline()) {
                move(EDIT_UP, EDIT_EXTEND_UP, shift);
              } else {
                host.focusStep("up");
              }
              return;
            }
            if (name === "Down") {
              if (multiline()) {
                move(EDIT_DOWN, EDIT_EXTEND_DOWN, shift);
              } else {
                host.focusStep("down");
              }
              return;
            }
            if (name === "Home") {
              move(EDIT_LINE_START, EDIT_EXTEND_LINE_START, shift);
              return;
            }
            if (name === "End") {
              move(EDIT_LINE_END, EDIT_EXTEND_LINE_END, shift);
              return;
            }
            if (name === "WordLeft") {
              move(EDIT_WORD_LEFT, EDIT_EXTEND_WORD_LEFT, shift);
              return;
            }
            if (name === "WordRight") {
              move(EDIT_WORD_RIGHT, EDIT_EXTEND_WORD_RIGHT, shift);
              return;
            }
            if (name === "DocumentStart") {
              move(EDIT_DOCUMENT_START, EDIT_EXTEND_DOCUMENT_START, shift);
              return;
            }
            if (name === "DocumentEnd") {
              move(EDIT_DOCUMENT_END, EDIT_EXTEND_DOCUMENT_END, shift);
              return;
            }
            if (name === "SelectAll") {
              apply(EDIT_SELECT_ALL, "", "motion");
              publish(true);
              return;
            }
            if (name === "Copy") {
              if (password()) {
                return;
              }
              apply(EDIT_COPY, "", "motion");
              return;
            }
            if (name === "Cut") {
              if (password()) {
                return;
              }
              apply(EDIT_CUT, "", "change");
              publish(true);
              return;
            }
            if (name === "Backspace") {
              apply(EDIT_BACKSPACE, "", "change");
              publish(true);
              return;
            }
            if (name === "Delete") {
              apply(EDIT_FORWARD_DELETE, "", "change");
              publish(true);
              return;
            }
            if (name === "DeleteWord") {
              apply(EDIT_DELETE_WORD, "", "change");
              publish(true);
              return;
            }
            if (name === "DeleteWordForward") {
              apply(EDIT_DELETE_WORD_FORWARD, "", "change");
              publish(true);
              return;
            }
            if (name === "DeleteToLineStart") {
              apply(EDIT_DELETE_TO_LINE_START, "", "change");
              publish(true);
              return;
            }
            if (name === "DeleteToLineEnd") {
              apply(EDIT_DELETE_TO_LINE_END, "", "change");
              publish(true);
              return;
            }
            if (name === "Enter") {
              if (submitEnter()) {
                props.onSubmit?.(text());
              } else {
                apply(EDIT_NEWLINE, "", "change");
                publish(true);
              }
            }
          }}
          onText={(value) => {
            setComposing(null);
            apply(EDIT_INSERT, value, "change");
            publish(true);
          }}
          onTextReplaced={(value) => {
            setComposing(null);
            setText(value);
            const end = utf16Len(value);
            setBase(end);
            setExtent(end);
            paint();
            props.onChange?.(value);
          }}
          onComposing={(value) => {
            setComposing(value);
            if (value == null) {
              apply(EDIT_COMPOSING, "", "change");
            } else {
              apply(EDIT_COMPOSING, value, "compose");
            }
            publish(true);
          }}
        >
          <padding edges={theme().space.s}>
            <decorated
              color={shownError() ? theme().color.danger : focused() ? theme().color.accent : theme().color.textMuted}
              radius={theme().radius.m}
              fill
              min={{ w: 280, h: 48 }}
            >
              <padding edges={1}>
                <decorated color={theme().color.surface} radius={theme().radius.s} fill>
                  <padding edges={10}>
                    <scroll
                      ref={(handle) => {
                        view = handle.id;
                      }}
                    >
                      <padding right={2}>
                        <stack>
                          <Text
                            fontSize={theme().fontSize.body}
                            color={hint() ? theme().color.textMuted : theme().color.text}
                            weight={theme().weight.body}
                            direction={directionFor(theme().locale)}
                            softWrap={multiline()}
                            overflow={multiline() ? "clip" : "visible"}
                            maxLines={multiline() ? props.maxLines : undefined}
                          >
                            {shown()}
                          </Text>
                          {rects().map((rect) => (
                            <transform x={rect.x} y={rect.y}>
                              <decorated color={`${theme().color.accent}66`}>
                                <sized w={rect.w} h={rect.h} />
                              </decorated>
                            </transform>
                          ))}
                          <transform x={caretX()} y={caretY()}>
                            <decorated
                              color={
                                focused() && blink() && base() === extent()
                                  ? theme().color.accent
                                  : "#00000000"
                              }
                            >
                              <sized w={2} h={caretH()} />
                            </decorated>
                          </transform>
                        </stack>
                      </padding>
                    </scroll>
                  </padding>
                </decorated>
              </padding>
            </decorated>
          </padding>
        </pointer>
      </opacity>
      {shownError() ? (
        <Text
          fontSize={theme().fontSize.body}
          color={theme().color.danger}
          weight={theme().weight.body}
          direction={directionFor(theme().locale)}
        >
          {shownError()}
        </Text>
      ) : null}
    </column>
  );
}
