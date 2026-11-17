import { readFileSync } from "node:fs";
import * as esbuild from "esbuild";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { assertNoScheduler, packOptions } from "../../tools/pack.mjs";

const here = dirname(fileURLToPath(import.meta.url));

const options = packOptions({
  absWorkingDir: here,
  entryPoints: ["src/quotes.tsx"],
  outfile: "dist/quotes.js",
});

if (process.argv.includes("--watch")) {
  const ctx = await esbuild.context(options);
  await ctx.watch();
} else {
  await esbuild.build(options);
  const packed = readFileSync(join(here, "dist/quotes.js"), "utf8");
  assertNoScheduler(packed);
}
