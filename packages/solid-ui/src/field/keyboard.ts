/** Text-edit opcodes. 38 is Commit: replace the composing range, then insert. Load is 0 and clears undo. */

export const EDIT_LOAD = 0;
export const EDIT_LEFT = 1;
export const EDIT_RIGHT = 2;
export const EDIT_BACKSPACE = 3;
export const EDIT_INSERT = 4;
export const EDIT_COMPOSING = 5;
export const EDIT_UNDO = 7;
export const EDIT_REDO = 8;
export const EDIT_SELECT_ALL = 9;
export const EDIT_COPY = 10;
export const EDIT_CUT = 11;
export const EDIT_CARET = 12;
export const EDIT_FORWARD_DELETE = 13;
export const EDIT_NEWLINE = 14;
export const EDIT_WORD_LEFT = 15;
export const EDIT_WORD_RIGHT = 16;
export const EDIT_LINE_START = 17;
export const EDIT_LINE_END = 18;
export const EDIT_EXTEND_LEFT = 19;
export const EDIT_EXTEND_RIGHT = 20;
export const EDIT_EXTEND_WORD_LEFT = 21;
export const EDIT_EXTEND_WORD_RIGHT = 22;
export const EDIT_EXTEND_LINE_START = 23;
export const EDIT_EXTEND_LINE_END = 24;
export const EDIT_UP = 25;
export const EDIT_DOWN = 26;
export const EDIT_EXTEND_UP = 27;
export const EDIT_EXTEND_DOWN = 28;
export const EDIT_EXTEND_CARET = 29;
export const EDIT_DELETE_WORD = 30;
export const EDIT_DELETE_WORD_FORWARD = 31;
export const EDIT_DELETE_TO_LINE_START = 32;
export const EDIT_DELETE_TO_LINE_END = 33;
export const EDIT_DOCUMENT_START = 34;
export const EDIT_DOCUMENT_END = 35;
export const EDIT_EXTEND_DOCUMENT_START = 36;
export const EDIT_EXTEND_DOCUMENT_END = 37;
export const EDIT_COMMIT = 38;

export const KEYBOARD: Record<string, number> = {
  text: 0,
  multiline: 1,
  number: 2,
  phone: 3,
  email: 4,
  url: 5,
  name: 6,
  password: 7,
};

export const ACTION: Record<string, number> = {
  none: 0,
  done: 1,
  next: 2,
  search: 3,
  go: 4,
  send: 5,
  previous: 6,
  newline: 7,
};

export const SUBMIT_ACTIONS = new Set(["done", "search", "go", "send", "next"]);

export type KeyboardName = "text" | "number" | "phone" | "email" | "url" | "name" | "password";
export type ActionName = "none" | "done" | "next" | "search" | "go" | "send" | "previous" | "newline";

/** keyboard × action codes passed to `setImeConfig`. Multiline defaults to the newline action. */
export function imeConfig(
  keyboard: KeyboardName | undefined,
  action: ActionName | undefined,
  multiline: boolean,
): { keyboard: number; action: number } {
  const keyboardCode = keyboard && keyboard !== "text" ? (KEYBOARD[keyboard] ?? 0) : multiline ? 1 : 0;
  let actionCode: number;
  if (action) {
    actionCode = ACTION[action] ?? 0;
  } else {
    actionCode = multiline ? ACTION.newline : ACTION.done;
  }
  return { keyboard: keyboardCode ?? 0, action: actionCode ?? 0 };
}

/** Enter inserts a newline only for a multiline field whose action is not a submit action. */
export function enterInsertsNewline(multiline: boolean, action: ActionName | undefined): boolean {
  if (!multiline) {
    return false;
  }
  if (action !== undefined && SUBMIT_ACTIONS.has(action)) {
    return false;
  }
  return true;
}
