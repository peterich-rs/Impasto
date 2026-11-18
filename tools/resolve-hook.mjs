import { existsSync, readdirSync, readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const require = createRequire(import.meta.url);

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const packagesRoot = join(root, "packages");

// @impasto/<pkg>/<sub> -> packages/<pkg>/<exports 值>，数据源是各包自己的 package.json
const files = {};
const capabilityPackages = [];
for (const entry of readdirSync(packagesRoot, { withFileTypes: true })) {
  if (!entry.isDirectory() || entry.name.startsWith(".")) continue;
  if (!existsSync(join(packagesRoot, entry.name, "package.json"))) continue;
  const manifest = JSON.parse(
    readFileSync(join(packagesRoot, entry.name, "package.json"), "utf8"),
  );
  for (const [key, value] of Object.entries(manifest.exports ?? {})) {
    const subpath = key === "." ? "" : key.replace(/^\.\//, "");
    files[`@impasto/${entry.name}${subpath ? `/${subpath}` : ""}`] = `${entry.name}/${value}`;
  }
  // The package entry is a typed wrapper. Node tests have no native module, so
  // they load the stub instead. The declaration file next to it is types only.
  const stub = join(packagesRoot, entry.name, "src", "native.stub.ts");
  if (existsSync(stub)) {

    capabilityPackages.push(entry.name);
  }
}

files["@impasto/reactive"]="reactive/testing/reference.ts";

function capabilityPackageOf(parentURL) {
  if (!parentURL) {
    return null;
  }
  const parent = fileURLToPath(parentURL);
  for (const pkg of capabilityPackages) {
    const prefix = join(packagesRoot, pkg, "src");
    if (parent === prefix || parent.startsWith(`${prefix}/`)) {
      return pkg;
    }
  }
  return null;
}

export function resolve(specifier, context, nextResolve) {
  if (specifier === "solid-js" && !context.parentURL?.endsWith("/reactive/testing/reference.ts")) return { url: pathToFileURL(join(packagesRoot, "reactive/testing/reference.ts")).href, shortCircuit: true };
  if (specifier === "scene") return { url: pathToFileURL(join(packagesRoot,"solid-renderer/testing/native.ts")).href, shortCircuit:true };
  if (specifier === "reactive") return { url:pathToFileURL(join(packagesRoot,"reactive/testing/reference.ts")).href,shortCircuit:true };
  if (context.parentURL?.includes("/dist/") && /^@impasto\/(clipboard|linking|async-storage|app-state|permissions|file-system|devtools)$/.test(specifier)) return {url:pathToFileURL(join(packagesRoot,specifier.slice(9),"src/native.stub.ts")).href,shortCircuit:true};
  const relative = files[specifier];
  if (relative) {
    return {
      url: pathToFileURL(join(packagesRoot, relative)).href,
      shortCircuit: true,
    };
  }
  // A package's own index imports its declaration file. Tests load the stub.
  const pkg = capabilityPackageOf(context.parentURL);
  if (pkg && /(?:devtools|clipboard|linking|files|storage|app-state|permissions)\.ts$/.test(specifier)) return { url:pathToFileURL(join(packagesRoot,pkg,"src/native.stub.ts")).href,shortCircuit:true };
  return nextResolve(specifier, context);
}

// Node's type stripper loads `.ts` only, so `.tsx` sources are stripped here.
export function load(url, context, nextLoad) {
  if (!url.startsWith("file:") || !url.endsWith(".tsx")) {
    return nextLoad(url, context);
  }
  const { stripTypeScriptTypes } = require("node:module");
  const source = stripTypeScriptTypes(readFileSync(fileURLToPath(url), "utf8"), { mode: "strip" });
  return { format: "module", source, shortCircuit: true };
}
