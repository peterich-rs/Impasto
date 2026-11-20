import { transformAsync } from "@babel/core";
import transformTypescript from "@babel/plugin-transform-typescript";
import solidTransform from "@solidjs/babel-plugin";
import * as esbuild from "esbuild";
import { readFileSync, existsSync, realpathSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const packages = join(root, "packages");

export function assertNoScheduler(code, inputs = []) {
  if (inputs.some(path => /node_modules\/.*(?:solid-js|@solidjs[+/](?:signals|universal))/.test(path)) || /REACTIVITY_HALTED|@solidjs\/universal/.test(code)) {
    throw new Error("bundle contains a Solid scheduler or @solidjs/universal");
  }
}

export function packOptions({ absWorkingDir, entryPoints, outfile, release = false, sourceMap = !release }) {
  return {
    absWorkingDir, entryPoints, outfile, bundle: true, format: "esm",
    platform: "neutral", conditions: ["browser"], target: "es2022",
    sourcemap: sourceMap, minify: release, metafile: true,
    external: ["scene", "reactive"],
    define: { "process.env.NODE_ENV": JSON.stringify(release ? "production" : "development") },
    plugins: [{
      name: "impasto",
      setup(build) {
        build.onResolve({ filter: /^solid-js$/ }, () => ({ path: join(packages, "reactive/src/index.ts") }));
        build.onResolve({ filter: /^@impasto\// }, args => {
          const [name, ...rest] = args.path.slice(9).split("/");
          const manifest = join(packages, name, "package.json");
          if (!existsSync(manifest)) return;
          const exports = JSON.parse(readFileSync(manifest, "utf8")).exports;
          const target = exports?.[rest.length ? `./${rest.join('/')}` : "."];
          if (target === "./src/native.d.ts") return { path: args.path, external: true };
          return typeof target === "string" ? { path: join(packages, name, target) } : undefined;
        });
        build.onResolve({ filter: /^\.\.?\/(.*\/)?native\.js$/ }, args => {
          const source = dirname(resolve(args.resolveDir, args.path));
          const folder = dirname(source);
          if (dirname(folder) !== packages || basename(source) !== "src") return;
          if (!existsSync(join(source, "native.d.ts"))) return;
          return { path: `@impasto/${basename(folder)}`, external: true };
        });
        build.onLoad({ filter: /\.tsx?$/ }, async args => {
          const result = await transformAsync(readFileSync(args.path, "utf8"), {
            filename: args.path, babelrc: false, configFile: false,
            sourceMaps: sourceMap ? "inline" : false,
            plugins: [
              [transformTypescript, { isTSX: true, allExtensions: true }],
              [solidTransform, { moduleName: "@impasto/solid-renderer", generate: "universal" }],
            ],
          });
          if (result?.code == null) throw new Error(`Babel produced no code for ${args.path}`);
          return { contents: result.code, loader: "js" };
        });
        build.onEnd(result => {
          if (result.errors.length) return;
          assertNoScheduler(readFileSync(resolve(absWorkingDir, outfile), "utf8"), Object.keys(result.metafile.inputs));
        });
      },
    }],
    logLevel: "info",
  };
}
if (process.argv[1] && realpathSync(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [entry, outfile, ...flags] = process.argv.slice(2);
  if (!entry || !outfile) throw new Error("usage: pack <entry.tsx> <outfile.js> [--release] [--source-map]");
  const release = flags.includes("--release");
  const sourceMap = flags.includes("--source-map") ? "external" : !release;
  await esbuild.build(packOptions({ absWorkingDir: dirname(resolve(entry)), entryPoints: [resolve(entry)], outfile: resolve(outfile), release, sourceMap }));
}
