// Observable order of Solid 2.0.0-rc.17. Run from the gallery app, which depends
// on this exact version:
//   node crates/reactive/tests/fixtures/order.mjs
// The Rust tests assert the same lines. This file does not get copied into the crate.
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { runProbe } from "./probe.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const require = createRequire(join(here, "../../../../packages/reactive/package.json"));
const solid = require("solid-js");

const lines = [];
runProbe((line) => { lines.push(line); }, solid);
process.stdout.write(`${lines.join("\n")}\n`);
