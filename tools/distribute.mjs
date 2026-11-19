// Build a relocatable CLI toolchain with its own Node runtime. Native builds
// continue to use Cargo and the platform SDK; bundle/analyze/test need no Node
// installation on the application machine.
import { cpSync,copyFileSync,existsSync,mkdirSync,readFileSync,writeFileSync,rmSync } from "node:fs";
import { dirname,join,relative,resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
const root=resolve(dirname(fileURLToPath(import.meta.url)),"..");
const output=resolve(process.argv[2] ?? join(root,"target/cli-dist"));
if(output===root || root.startsWith(output+"/")) throw new Error("distribution output must not contain the checkout");
const cargo=spawnSync("cargo",["build","--release","-p","cli"],{cwd:root,stdio:"inherit"});
if(cargo.status!==0) process.exit(cargo.status ?? 1);
let macosSdk;
if(process.platform==="darwin") {
  const prepared=spawnSync("python3",[join(root,"tools/impeller_sdk.py"),"prepare"],{
    cwd:root,encoding:"utf8",stdio:["ignore","pipe","inherit"],
  });
  if(prepared.status!==0) process.exit(prepared.status ?? 1);
  macosSdk=prepared.stdout.trim();
  const host=spawnSync("cargo",["build","--release","-p","macos","--features","impeller"],{
    cwd:root,stdio:"inherit",env:{...process.env,IMPASTO_IMPELLER_SDK_DIR:macosSdk},
  });
  if(host.status!==0) process.exit(host.status ?? 1);
}
mkdirSync(output,{recursive:true});copyFileSync(join(root,"target/release/impasto"),join(output,"impasto"));
const tools=join(output,"impasto-tools");const framework=join(tools,"framework");rmSync(framework,{recursive:true,force:true});mkdirSync(framework,{recursive:true});
const pin=readFileSync(join(root,"crates/cli/toolchain.pin"),"utf8");
const version=pin.match(/^version (\S+)/m)?.[1];
const platform=`${process.platform}-${process.arch}`;
const digest=pin.split("\n").map(line=>line.split(" ")).find(parts=>parts[0]==="platform" && parts[1]===platform)?.[2];
if(!version || !digest) throw new Error(`no pinned Node runtime for ${platform}`);
const name=`node-v${version}-${platform}`;
const archive=join(root,"target",`${name}.tar.xz`);
const valid=()=>existsSync(archive) && createHash("sha256").update(readFileSync(archive)).digest("hex")===digest;
if(!valid()) {
  const response=await fetch(`https://nodejs.org/dist/v${version}/${name}.tar.xz`);
  if(!response.ok) throw new Error(`Node download: ${response.status}`);
  writeFileSync(archive,Buffer.from(await response.arrayBuffer()));
  if(!valid()) {rmSync(archive);throw new Error("Node runtime sha256 mismatch");}
}
const unpack=join(root,"target/toolchain-node");mkdirSync(unpack,{recursive:true});
const tar=spawnSync("tar",["-xJf",archive,"-C",unpack,`${name}/bin/node`,`${name}/LICENSE`],{stdio:"inherit"});
if(tar.status!==0) process.exit(tar.status ?? 1);
rmSync(join(tools,"node"),{force:true});
copyFileSync(join(unpack,name,"bin/node"),join(tools,"node"));
copyFileSync(join(unpack,name,"LICENSE"),join(tools,"LICENSE.node"));
const copy=(from,to)=>cpSync(from,to,{recursive:true,dereference:false,verbatimSymlinks:true,filter:path=>!['.git','.gradle','__pycache__','node_modules','target','build','dist'].includes(path.split('/').at(-1))});
for(const name of ["tools","packages","crates","Cargo.toml","Cargo.lock","rust-toolchain.toml","LICENSE","NOTICE","package.json","pnpm-lock.yaml","pnpm-workspace.yaml"]){copy(join(root,name),join(framework,name));}
// Preserve pnpm's relative links inside the self-contained tree. This ships the
// published Babel/Solid compiler, esbuild binary and TypeScript, without source vendoring.
cpSync(join(root,"node_modules"),join(framework,"node_modules"),{recursive:true,dereference:false,verbatimSymlinks:true,filter:path=>!path.includes('/.cache/')});
for(const pkg of ["reactive","solid-renderer"]){
  const from=join(root,"packages",pkg,"node_modules");if(existsSync(from))cpSync(from,join(framework,"packages",pkg,"node_modules"),{recursive:true,dereference:false,verbatimSymlinks:true});
}
if(macosSdk) {
  copy(macosSdk,join(framework,relative(root,macosSdk)));
  mkdirSync(join(framework,"target/release"),{recursive:true});
  mkdirSync(join(framework,"target/Frameworks"),{recursive:true});
  copyFileSync(join(root,"target/release/macos"),join(framework,"target/release/macos"));
  copyFileSync(join(macosSdk,"lib/libimpeller.dylib"),join(framework,"target/Frameworks/libimpeller.dylib"));
}
console.log(output);
