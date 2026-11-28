import { assert, test } from "impasto:test";
import { openDatabase } from "../src/index";
import { sql } from "../src/query";

test("rolls back a transaction and preserves exact 64-bit parameters", async () => {
  const database = await openDatabase({ name: ":memory:", int64Mode: "bigint" });
  try {
    await database.execute("CREATE TABLE notes(id INTEGER PRIMARY KEY, title TEXT)");
    try {
      await database.transaction(async tx => {
        await tx.execute("INSERT INTO notes VALUES (?, ?)", [1n, "rolled back"]);
        throw new Error("rollback");
      });
    } catch (error) { assert.equal(String(error).includes("rollback"), true); }
    assert.equal(await database.queryOne("SELECT title FROM notes"), null);
    const title = "quote ' ; DROP TABLE notes;";
    const insert = sql`INSERT INTO notes VALUES (${9_007_199_254_740_993n}, ${title})`;
    await database.execute(insert.text, insert.params);
    assert.deepEqual(await database.queryOne("SELECT id, title FROM notes"), { id: 9_007_199_254_740_993n, title });
  } finally { await database.close(); }
});

test("deleting a database waits for the application to close its handles", async () => {
  const { deleteDatabase } = await import("../src/index");
  const name = "locked-notes.sqlite";
  const database = await openDatabase({ name });
  try {
    await database.execute("CREATE TABLE notes (title TEXT)");
    await assert.rejects(() => deleteDatabase(name), (error: unknown) => String(error).includes("busy"));
    await database.transaction(async tx => {
      await assert.rejects(() => database.execute("DELETE FROM notes"), (error: unknown) => String(error).includes("busy"));
      await tx.execute("INSERT INTO notes VALUES (?)", ["finished"]);
    });
  } finally { await database.close(); }
  await deleteDatabase(name);
});
