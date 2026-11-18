import * as module from "node:module";
import { load, resolve } from "./resolve-hook.mjs";

if (typeof module.registerHooks === "function") {
  module.registerHooks({ load, resolve });
} else {
  module.register(new URL("./resolve-hook.mjs", import.meta.url));
}
