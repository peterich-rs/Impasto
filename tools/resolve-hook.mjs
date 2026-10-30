import { readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const packagesRoot = join(root, "packages");

// @impasto/<pkg>/<sub> -> packages/<pkg>/<exports 值>，数据源是各包自己的 package.json
const files = {};
const hostPackages = [];
for (const entry of readdirSync(packagesRoot, { withFileTypes: true })) {
  if (!entry.isDirectory() || entry.name.startsWith(".")) continue;
  const manifest = JSON.parse(
    readFileSync(join(packagesRoot, entry.name, "package.json"), "utf8"),
  );
  for (const [key, value] of Object.entries(manifest.exports ?? {})) {
    const subpath = key === "." ? "" : key.replace(/^\.\//, "");
    files[`@impasto/${entry.name}${subpath ? `/${subpath}` : ""}`] = `${entry.name}/${value}`;
  }
  // `./host` 在 node 测试里落到 stub；exports 指向的 host.gen.ts 只是声明。
  if (files[`@impasto/${entry.name}/host`]) {
    files[`@impasto/${entry.name}/host`] = `${entry.name}/src/host.stub.ts`;
    hostPackages.push(entry.name);
  }
}

function hostPackageOf(parentURL) {
  if (!parentURL) {
    return null;
  }
  const parent = fileURLToPath(parentURL);
  for (const pkg of hostPackages) {
    const prefix = join(packagesRoot, pkg, "src");
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
      url: pathToFileURL(join(packagesRoot, relative)).href,
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
        url: pathToFileURL(join(packagesRoot, pkg, "src", "host.stub.ts")).href,
        shortCircuit: true,
      };
    }
  }
  return nextResolve(specifier, context);
}
