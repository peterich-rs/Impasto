import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = dirname(fileURLToPath(import.meta.url));

const files = {
  "@uifw/modules": "modules/src/index.ts",
  "@uifw/solid-renderer": "solid-renderer/runtime/index.ts",
  "@uifw/solid-renderer/defaults": "solid-renderer/src/defaults.ts",
  "@uifw/solid-renderer/jsx": "solid-renderer/src/jsx.ts",
  "@uifw/solid-renderer/palette": "solid-renderer/src/palette.ts",
  "@uifw/solid-router": "solid-router/src/index.ts",
  "@uifw/solid-ui": "solid-ui/src/index.tsx",
  "@uifw/net": "net/src/index.ts",
  "@uifw/form": "form/src/index.ts",
  "@uifw/i18n": "i18n/src/index.ts",
};

export function resolve(specifier, context, nextResolve) {
  const relative = files[specifier];
  if (relative) {
    return {
      url: pathToFileURL(join(root, relative)).href,
      shortCircuit: true,
    };
  }
  return nextResolve(specifier, context);
}
