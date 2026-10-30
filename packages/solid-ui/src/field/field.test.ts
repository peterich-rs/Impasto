import assert from "node:assert/strict";
import { flush } from "solid-js";
import { createFieldController } from "./controller.ts";
import { EDIT_CARET, EDIT_COMMIT, EDIT_EXTEND_CARET, EDIT_LOAD, EDIT_NEWLINE } from "./keyboard.ts";
import { currency, decimal, phone } from "./mask.ts";

assert.equal(decimal().format("-12.3.4"), "-12.34");
assert.equal(phone().format("5551234567"), "(555) 123-4567");
assert.equal(phone().format("555"), "(555");
assert.equal(currency().format("1234.5"), "$1,234.50");
assert.equal(currency().format("-8"), "-$8.00");

let body = "";
const undo: string[] = [];
const ops: number[] = [];

function textEdit(op: number, extra: string) {
  ops.push(op);
  if (op === 5) {
    const shown = body + extra;
    return { text: shown, start: shown.length, end: shown.length };
  }
  if (op === EDIT_COMMIT) {
    undo.push(body);
    body = extra;
    return { text: body, start: body.length, end: body.length };
  }
  if (op === EDIT_LOAD) {
    undo.length = 0;
    body = extra;
    return { text: body, start: body.length, end: body.length };
  }
  if (op === 7 || op === 3) {
    const previous = undo.pop();
    if (previous !== undefined) {
      body = previous;
    }
    return { text: body, start: body.length, end: body.length };
  }
  if (op === EDIT_NEWLINE) {
    body += "\n";
    return { text: body, start: body.length, end: body.length };
  }
  return { text: body, start: body.length, end: body.length };
}

const field = createFieldController({ textEdit });
field.setMask(phone());
flush();
ops.length = 0;
field.onComposing("5551234567");
flush();
assert.equal(field.text(), "5551234567");
assert.equal(field.composing(), "5551234567");
assert.ok(ops.every((op) => op === 5));
assert.equal(ops.includes(EDIT_COMMIT), false);
field.onText("5551234567");
flush();
assert.equal(field.text(), "(555) 123-4567");
assert.equal(field.composing(), null);
assert.equal(ops.includes(EDIT_LOAD), false);
assert.equal(ops.at(-1), EDIT_COMMIT);
const formatted = field.text();
field.handleKey("Backspace", false, false, false, () => {});
flush();
assert.notEqual(field.text(), formatted);
assert.equal(ops.at(-1), 3);
assert.equal(ops.includes(EDIT_LOAD), false);

let submitted = false;
field.handleKey("Enter", true, false, false, () => {
  submitted = true;
});
flush();
assert.equal(submitted, false);
assert.equal(field.text().endsWith("\n"), true);
field.handleKey("Enter", true, false, true, () => {
  submitted = true;
});
assert.equal(submitted, true);
field.handleKey("Enter", false, false, false, () => {});

let buffer = "";
let anchor = 0;
let caret = 0;
const changes: string[] = [];
function editing(op: number, extra: string) {
  if (op === EDIT_LOAD) {
    buffer = extra;
    anchor = buffer.length;
    caret = buffer.length;
  } else if (op === EDIT_CARET) {
    anchor = Number(extra);
    caret = anchor;
  } else if (op === EDIT_EXTEND_CARET) {
    caret = Number(extra);
  }
  return { text: buffer, start: anchor, end: caret };
}
const synced = createFieldController({
  textEdit: editing,
  onChange: (value) => changes.push(value),
});
synced.load("还是旧的");
flush();
assert.equal(synced.text(), "还是旧的");
assert.equal(changes.length, 0);
synced.replace("还是旧", 3, 3, -1, -1);
flush();
assert.equal(synced.text(), "还是旧");
assert.deepEqual(synced.selection(), { start: 3, end: 3 });
assert.deepEqual(changes, ["还是旧"]);
synced.replace("还是旧的还是旧的", 8, 8, -1, -1);
flush();
assert.equal(changes.at(-1), "还是旧的还是旧的");
synced.replace("还是旧的还是旧的", 4, 4, -1, -1);
flush();
assert.equal(changes.length, 2);
assert.equal(synced.selection().start, 4);

console.log("field.test.ts ok");
