import { createSignal, type Accessor } from "solid-js";
import {
  EDIT_BACKSPACE,
  EDIT_CARET,
  EDIT_COMMIT,
  EDIT_COMPOSING,
  EDIT_COPY,
  EDIT_CUT,
  EDIT_DOCUMENT_END,
  EDIT_DOCUMENT_START,
  EDIT_DOWN,
  EDIT_EXTEND_CARET,
  EDIT_EXTEND_DOCUMENT_END,
  EDIT_EXTEND_DOCUMENT_START,
  EDIT_EXTEND_DOWN,
  EDIT_EXTEND_LEFT,
  EDIT_EXTEND_LINE_END,
  EDIT_EXTEND_LINE_START,
  EDIT_EXTEND_RIGHT,
  EDIT_EXTEND_UP,
  EDIT_EXTEND_WORD_LEFT,
  EDIT_EXTEND_WORD_RIGHT,
  EDIT_FORWARD_DELETE,
  EDIT_INSERT,
  EDIT_LEFT,
  EDIT_LINE_END,
  EDIT_LINE_START,
  EDIT_LOAD,
  EDIT_NEWLINE,
  EDIT_REDO,
  EDIT_RIGHT,
  EDIT_SELECT_ALL,
  EDIT_UNDO,
  EDIT_UP,
  EDIT_WORD_LEFT,
  EDIT_WORD_RIGHT,
  EDIT_DELETE_TO_LINE_END,
  EDIT_DELETE_TO_LINE_START,
  EDIT_DELETE_WORD,
  EDIT_DELETE_WORD_FORWARD,
} from "./keyboard.ts";
import type { InputMask } from "./mask.ts";

export type TextEditResult = { text: string; start: number; end: number };

export type Selection = { start: number; end: number };

export type FieldController = {
  text: Accessor<string>;
  selection: Accessor<Selection>;
  composing: Accessor<string | null>;
  /** UTF-16 composing span. `-1` when the buffer has no marked range. */
  composeStart: Accessor<number>;
  composeEnd: Accessor<number>;
  error: Accessor<string>;
  select(start: number, end: number): void;
  selectAll(): void;
  insert(value: string): void;
  undo(): void;
  redo(): void;
  clear(): void;
  setMask(mask: InputMask | null): void;
  setTransform(fn: ((value: string) => string) | null): void;
  focus(): void;
  /** IME composition. The raw string is stored; masks do not run. */
  onComposing(value: string | null): void;
  /** IME commit. A mask or transform is written with opcode 38, never Load. */
  onText(value: string): void;
  onTextReplaced(value: string): void;
  load(value: string): void;
  move(collapsed: number, extend: number, shift: boolean): void;
  apply(op: number, extra: string, kind: EditKind): void;
  handleKey(key: string, multiline: boolean, password: boolean, submitEnter: boolean, onSubmit: () => void): void;
  caretAt(unit: number): void;
  extendCaret(unit: number): void;
  /**
   * Install the input connection's buffer.
   * `anchor` and `caret` are UTF-16. A composing start below zero clears the span.
   */
  replace(text: string, anchor: number, caret: number, composeFrom: number, composeTo: number): void;
};

export type EditKind = "load" | "motion" | "change" | "compose";

function utf16Len(value: string): number {
  let count = 0;
  for (const char of value) {
    count += (char.codePointAt(0) ?? 0) > 0xffff ? 2 : 1;
  }
  return count;
}

export function createFieldController(options: {
  textEdit: (op: number, extra: string) => TextEditResult;
  onChange?: (value: string) => void;
  validate?: (value: string) => string | null | undefined;
  maxLength?: () => number | undefined;
  focus?: () => void;
  onVertical?: (direction: "up" | "down") => void;
  allowChange?: () => boolean;
}): FieldController {
  const [text, setText] = createSignal("");
  const [composing, setComposing] = createSignal<string | null>(null);
  const [composeStart, setComposeStart] = createSignal(-1);
  const [composeEnd, setComposeEnd] = createSignal(-1);
  const [start, setStart] = createSignal(0);
  const [end, setEnd] = createSignal(0);
  const [validError, setValidError] = createSignal("");
  let mask: InputMask | null = null;
  let transform: ((value: string) => string) | null = null;

  const selection = (): Selection => ({ start: start(), end: end() });

  function formatted(value: string): string {
    let next = value;
    if (mask) {
      next = mask.format(next);
    }
    if (transform) {
      next = transform(next);
    }
    return next;
  }

  function usesFormat(): boolean {
    return mask !== null || transform !== null;
  }

  function publishChange(before: string, next: string): void {
    if (next === before) {
      return;
    }
    options.onChange?.(next);
    if (options.validate) {
      setValidError(options.validate(next) ?? "");
    } else {
      setValidError("");
    }
  }

  function apply(op: number, extra: string, kind: EditKind): void {
    if ((kind === "change" || kind === "compose") && options.allowChange && !options.allowChange()) {
      return;
    }
    const before = text();
    const next = options.textEdit(op, extra);
    const limit = options.maxLength?.();
    if (kind !== "load" && limit !== undefined && utf16Len(next.text) > limit) {
      const reverted = options.textEdit(EDIT_LOAD, before);
      setText(reverted.text);
      setStart(reverted.start);
      setEnd(reverted.end);
      return;
    }
    setText(next.text);
    setStart(next.start);
    setEnd(next.end);
    if (kind === "change") {
      publishChange(before, next.text);
    }
  }

  function commitFormatted(raw: string): void {
    const next = formatted(raw);
    apply(EDIT_COMMIT, next, "change");
  }

  function rawBase(): string {
    const full = text();
    const extra = composing();
    if (extra === null || extra.length === 0) {
      return full;
    }
    if (full.endsWith(extra)) {
      return full.slice(0, full.length - extra.length);
    }
    return full;
  }

  const controller: FieldController = {
    text,
    selection,
    composing,
    composeStart,
    composeEnd,
    error: validError,
    select(from: number, to: number) {
      apply(EDIT_CARET, String(from), "motion");
      apply(EDIT_EXTEND_CARET, String(to), "motion");
    },
    selectAll() {
      apply(EDIT_SELECT_ALL, "", "motion");
    },
    insert(value: string) {
      if (usesFormat()) {
        const base = rawBase();
        commitFormatted(base + value);
        setComposing(null);
        return;
      }
      apply(EDIT_INSERT, value, "change");
    },
    undo() {
      apply(EDIT_UNDO, "", "change");
    },
    redo() {
      apply(EDIT_REDO, "", "change");
    },
    clear() {
      setComposing(null);
      if (usesFormat()) {
        apply(EDIT_COMMIT, formatted(""), "change");
        return;
      }
      apply(EDIT_SELECT_ALL, "", "motion");
      apply(EDIT_INSERT, "", "change");
    },
    setMask(next: InputMask | null) {
      mask = next;
      if (composing() !== null) {
        return;
      }
      if (next === null && transform === null) {
        return;
      }
      commitFormatted(text());
    },
    setTransform(next: ((value: string) => string) | null) {
      transform = next;
      if (composing() !== null) {
        return;
      }
      if (next === null && mask === null) {
        return;
      }
      commitFormatted(text());
    },
    focus() {
      options.focus?.();
    },
    onComposing(value: string | null) {
      if (value === null || value.length === 0) {
        setComposing(null);
        setComposeStart(-1);
        setComposeEnd(-1);
        apply(EDIT_COMPOSING, "", "change");
        return;
      }
      setComposing(value);
      apply(EDIT_COMPOSING, value, "compose");
      const caret = end();
      const units = utf16Len(value);
      setComposeStart(Math.max(0, caret - units));
      setComposeEnd(caret);
    },
    onText(value: string) {
      const raw = rawBase() + value;
      setComposing(null);
      if (usesFormat()) {
        commitFormatted(raw);
        return;
      }
      apply(EDIT_INSERT, value, "change");
    },
    onTextReplaced(value: string) {
      setComposing(null);
      if (usesFormat()) {
        commitFormatted(value);
        return;
      }
      setText(value);
      const caret = utf16Len(value);
      setStart(caret);
      setEnd(caret);
      options.onChange?.(value);
    },
    load(value: string) {
      if (value === text()) {
        return;
      }
      if (usesFormat()) {
        commitFormatted(value);
        return;
      }
      apply(EDIT_LOAD, value, "load");
    },
    move(collapsed: number, extend: number, shift: boolean) {
      apply(shift ? extend : collapsed, "", "motion");
    },
    apply,
    caretAt(unit: number) {
      apply(EDIT_CARET, String(unit), "motion");
    },
    replace(next: string, anchor: number, caret: number, composeFrom: number, composeTo: number) {
      const before = text();
      const limit = utf16Len(next);
      const marked =
        composeFrom >= 0 && composeTo > composeFrom && composeTo <= limit
          ? next.slice(composeFrom, composeTo)
          : "";
      if (marked.length > 0) {
        setComposing(marked);
        setComposeStart(composeFrom);
        setComposeEnd(composeTo);
      } else {
        setComposing(null);
        setComposeStart(-1);
        setComposeEnd(-1);
      }
      if (next !== before) {
        apply(EDIT_LOAD, next, "load");
        options.onChange?.(next);
      }
      const len = utf16Len(text());
      const from = Math.max(0, Math.min(anchor, len));
      const to = Math.max(from, Math.min(Math.max(anchor, caret), len));
      if (start() !== from || end() !== to) {
        apply(EDIT_CARET, String(from), "motion");
        if (to !== from) {
          apply(EDIT_EXTEND_CARET, String(to), "motion");
        }
      }
    },
    extendCaret(unit: number) {
      apply(EDIT_EXTEND_CARET, String(unit), "motion");
    },
    handleKey(key: string, multiline: boolean, password: boolean, submitEnter: boolean, onSubmit: () => void) {
      const shift = key.startsWith("Shift+");
      const name = shift ? key.slice("Shift+".length) : key;
      if (name === "Left") {
        controller.move(EDIT_LEFT, EDIT_EXTEND_LEFT, shift);
        return;
      }
      if (name === "Right") {
        controller.move(EDIT_RIGHT, EDIT_EXTEND_RIGHT, shift);
        return;
      }
      if (name === "Up") {
        if (multiline) {
          controller.move(EDIT_UP, EDIT_EXTEND_UP, shift);
        } else {
          options.onVertical?.("up");
        }
        return;
      }
      if (name === "Down") {
        if (multiline) {
          controller.move(EDIT_DOWN, EDIT_EXTEND_DOWN, shift);
        } else {
          options.onVertical?.("down");
        }
        return;
      }
      if (name === "Home") {
        controller.move(EDIT_LINE_START, EDIT_EXTEND_LINE_START, shift);
        return;
      }
      if (name === "End") {
        controller.move(EDIT_LINE_END, EDIT_EXTEND_LINE_END, shift);
        return;
      }
      if (name === "WordLeft") {
        controller.move(EDIT_WORD_LEFT, EDIT_EXTEND_WORD_LEFT, shift);
        return;
      }
      if (name === "WordRight") {
        controller.move(EDIT_WORD_RIGHT, EDIT_EXTEND_WORD_RIGHT, shift);
        return;
      }
      if (name === "DocumentStart") {
        controller.move(EDIT_DOCUMENT_START, EDIT_EXTEND_DOCUMENT_START, shift);
        return;
      }
      if (name === "DocumentEnd") {
        controller.move(EDIT_DOCUMENT_END, EDIT_EXTEND_DOCUMENT_END, shift);
        return;
      }
      if (name === "SelectAll") {
        controller.selectAll();
        return;
      }
      if (name === "Copy") {
        if (!password) {
          apply(EDIT_COPY, "", "motion");
        }
        return;
      }
      if (name === "Cut") {
        if (!password) {
          apply(EDIT_CUT, "", "change");
        }
        return;
      }
      if (name === "Backspace") {
        apply(EDIT_BACKSPACE, "", "change");
        return;
      }
      if (name === "Delete") {
        apply(EDIT_FORWARD_DELETE, "", "change");
        return;
      }
      if (name === "DeleteWord") {
        apply(EDIT_DELETE_WORD, "", "change");
        return;
      }
      if (name === "DeleteWordForward") {
        apply(EDIT_DELETE_WORD_FORWARD, "", "change");
        return;
      }
      if (name === "DeleteToLineStart") {
        apply(EDIT_DELETE_TO_LINE_START, "", "change");
        return;
      }
      if (name === "DeleteToLineEnd") {
        apply(EDIT_DELETE_TO_LINE_END, "", "change");
        return;
      }
      if (name === "Enter") {
        if (submitEnter) {
          onSubmit();
        } else {
          apply(EDIT_NEWLINE, "", "change");
        }
      }
    },
  };

  return controller;
}

export function fieldError(controllerError: string, override: string | undefined): string {
  if (override !== undefined && override.length > 0) {
    return override;
  }
  return controllerError;
}
