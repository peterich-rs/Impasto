import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = dirname(fileURLToPath(import.meta.url));

const hostPackages = [
  "clipboard",
  "linking",
  "app-state",
  "file-system",
  "async-storage",
  "permissions",
];

const files = {
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

for (const pkg of hostPackages) {
  files[`@uifw/${pkg}`] = `${pkg}/src/index.ts`;
  files[`@uifw/${pkg}/host`] = `${pkg}/src/host.stub.ts`;
}

function hostPackageOf(parentURL) {
  if (!parentURL) {
    return null;
  }
  const parent = fileURLToPath(parentURL);
  for (const pkg of hostPackages) {
    const prefix = join(root, pkg, "src");
    if (parent === prefix || parent.startsWith(`${prefix}/`)) {
      return pkg;
    }
  }
  return null;
}

export function resolve(specifier, context, nextResolve) {
  const relative = files[specifier];
  if (relative) {
    return {
      url: pathToFileURL(join(root, relative)).href,
      shortCircuit: true,
    };
  }
  if (
    specifier === "./host" ||
    specifier === "./host.ts" ||
    specifier === "./host.gen.ts"
  ) {
    const pkg = hostPackageOf(context.parentURL);
    if (pkg) {
      return {
        url: pathToFileURL(join(root, pkg, "src", "host.stub.ts")).href,
        shortCircuit: true,
      };
    }
  }
  return nextResolve(specifier, context);
}
