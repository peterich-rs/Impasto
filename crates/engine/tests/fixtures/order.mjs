// Observable order of Solid 2.0.0, installed by packages/reactive. Re-record with:
//   node --conditions=browser crates/engine/tests/fixtures/order.mjs > crates/engine/tests/fixtures/order.txt
// The Rust and QuickJS tests compare against the same output.
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { runProbe } from "./probe.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const require = createRequire(join(here, "../../../../packages/reactive/package.json"));
const { version } = require("solid-js/package.json");
if (version !== "2.0.0") throw new Error(`Expected Solid 2.0.0, found ${version}`);
const solid = require("solid-js");

const lines = [];
runProbe((line) => { lines.push(line); }, solid);
process.stdout.write(`${lines.join("\n")}\n`);
