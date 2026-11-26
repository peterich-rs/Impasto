import { assert, screen, test } from "impasto:test";
import { scene } from "@impasto/solid-renderer";
import { QuoteEngine } from "../src/engine/engine.ts";
import { QuoteBinder } from "../src/state/binder.ts";
import { fmtPct, fmtPrice } from "../src/format.ts";

const colors = {
  up: [235, 62, 62, 255],
  down: [34, 176, 95, 255],
  flat: [138, 141, 145, 255],
  base: [30, 34, 40, 255],
} satisfies ConstructorParameters<typeof QuoteBinder>[1];

function setup(seed: number) {
  const engine = new QuoteEngine({ timeScale: 1, seed });
  const binder = new QuoteBinder(engine, colors, () => "cn");
  const root = scene.createElement("view");
  const cell = () => {
    const node = scene.createTextNode("—");
    scene.insertNode(root, node);
    return node;
  };
  return { engine, binder, root, cell };
}

async function labels() {
  return new Map((await screen.nodes("text")).map(node => [node.id, node.label]));
}

test("independent binds write native text and stop after disposal", async () => {
  const { engine, binder, root, cell } = setup(1);
  const symbol = engine.symbols()[0]!;
  const last = cell();
  const pct = cell();
  const stopLast = binder.bind(symbol, { last, flashNode: last });
  const stopPct = binder.bind(symbol, { pct, flashNode: pct });
  await screen.mount(() => root);
  const initial = await labels();
  assert.equal(initial.get(last), fmtPrice(engine.quote(symbol).last));
  assert.ok(initial.get(pct) !== "—");

  engine.runFor(5_000);
  binder.flush();
  await screen.advanceFrames();
  const updated = await labels();
  assert.equal(updated.get(last), fmtPrice(engine.quote(symbol).last));
  stopLast();
  engine.runFor(5_000);
  binder.flush();
  await screen.advanceFrames();
  const remaining = await labels();
  assert.equal(remaining.get(last), updated.get(last));
  const quote = engine.quote(symbol);
  assert.equal(remaining.get(pct), fmtPct(quote.last, quote.prevClose));
  stopPct();

  const stale = cell();
  binder.bind(symbol, { last: stale, flashNode: stale });
  scene.remove(root, stale);
  assert.doesNotThrow(() => binder.flush());
});

test("the per-frame cap keeps a last and percent pair together", async () => {
  const { engine, binder, root, cell } = setup(3);
  const symbols = engine.symbols().slice(0, 10);
  for (const symbol of symbols.slice(0, 7)) binder.bind(symbol, { last: cell() });
  const last = cell();
  const pct = cell();
  binder.bind(symbols[7]!, { last, pct });
  for (const symbol of symbols.slice(8)) binder.bind(symbol, { last: cell() });
  await screen.mount(() => root);
  const initial = await labels();

  engine.runFor(5_000);
  binder.flush();
  await screen.advanceFrames();
  const first = await labels();
  assert.equal(first.get(last), initial.get(last));
  assert.equal(first.get(pct), initial.get(pct));
  assert.ok([...first].some(([id, text]) => initial.get(id) !== text));

  binder.flush();
  await screen.advanceFrames();
  const second = await labels();
  const quote = engine.quote(symbols[7]!);
  assert.equal(second.get(last), fmtPrice(quote.last));
  assert.equal(second.get(pct), fmtPct(quote.last, quote.prevClose));
});
