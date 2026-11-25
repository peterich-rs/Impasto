import ts from "typescript";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
const framework=resolve(dirname(fileURLToPath(import.meta.url)),"..");
const app=resolve(process.argv[2] ?? ".");
const packages=join(framework,"packages");
const paths={"solid-js":[join(packages,"reactive/src/types.ts")]};
for(const entry of readdirSync(packages,{withFileTypes:true})) {
  if(!entry.isDirectory() || !existsSync(join(packages,entry.name,"package.json"))) continue;
  const manifest=JSON.parse(readFileSync(join(packages,entry.name,"package.json"),"utf8"));
  for(const [key,value] of Object.entries(manifest.exports ?? {})) if(typeof value==="string") paths[manifest.name+(key==="." ? "" : key.slice(1))]=[join(packages,entry.name,value)];
}
// The author's imports retain official signatures; only runtime resolution is aliased.
paths["solid-js"]=[join(packages,"reactive/src/api.d.ts")];
paths["solid-js/jsx-runtime"]=[join(packages,"solid-renderer/src/jsx.ts")];
const config=ts.findConfigFile(app,ts.sys.fileExists);
const input=config ? ts.readConfigFile(config,ts.sys.readFile) : {config:{include:[join(app,existsSync(join(app,"lib")) ? "lib/**/*" : "src/**/*")]}};
if(input.error) throw new Error(ts.flattenDiagnosticMessageText(input.error.messageText,"\n"));
const parsed=ts.parseJsonConfigFileContent(input.config,ts.sys,config ? dirname(config) : app,{
  noEmit:true, target:ts.ScriptTarget.ES2022, module:ts.ModuleKind.ESNext,
  moduleResolution:ts.ModuleResolutionKind.Bundler, jsx:ts.JsxEmit.Preserve,
  jsxImportSource:"solid-js",allowImportingTsExtensions:true, skipLibCheck:true,
  strict:true, paths, baseUrl:framework,types:[],
});
const manifest=process.argv[3] ? JSON.parse(readFileSync(process.argv[3],"utf8")) : undefined;
if (manifest && manifest.schemaVersion !== 1) throw new Error("Unsupported plugins manifest schema");
for (const [name, entry] of Object.entries(manifest?.entries ?? {})) paths[name] = [entry];
const rootNames=[...parsed.fileNames,...(manifest?.types ?? []),join(packages,"reactive/src/modules.d.ts"),join(packages,"solid-renderer/src/jsx.ts")];
const program=ts.createProgram(rootNames,parsed.options);
const diagnostics=[...parsed.errors,...ts.getPreEmitDiagnostics(program)];
if(diagnostics.length) {
  process.stderr.write(ts.formatDiagnosticsWithColorAndContext(diagnostics,{getCanonicalFileName:p=>p,getCurrentDirectory:()=>app,getNewLine:()=>"\n"}));
  process.exitCode=1;
} else console.log(`TypeScript: ${parsed.fileNames.length} 个应用文件通过`);
