import * as module from "node:module";
import { resolve } from "./resolve-hook.mjs";

if (typeof module.registerHooks === "function") {
  module.registerHooks({ resolve });
} else {
  module.register(new URL("./resolve-hook.mjs", import.meta.url));
}
