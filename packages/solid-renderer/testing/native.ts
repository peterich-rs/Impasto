// Explicit Node-only test double. QuickJS integration tests exercise the native
// functions; this facade lets component tests inspect calls on their mock.
import { testScene } from "./scene.ts";
import { createSceneRenderer } from "./render.ts";
import type { SceneModule } from "../src/scene.ts";
let current: object | undefined;
let options: ReturnType<typeof testScene>;
let renderer: ReturnType<typeof createSceneRenderer>;
function source() { return (globalThis as { core?: SceneModule }).core; }
function refresh() {
  const next=source();
  if(next && next!==current) {current=next;options=testScene(next);renderer=createSceneRenderer(options);}
}
export const scene = new Proxy({} as SceneModule, {
  get(_target,key:string) {
    refresh();
    if(key==="createElement" || key==="createTextNode") return (...args:unknown[]) => {refresh();return (options[key] as Function)(...args);};
    if(key==="insert") return (...args:unknown[])=>{refresh();return (renderer.insert as Function)(...args);};
    if(key==="append") return (parent:number,child:unknown)=>{refresh();if(typeof child==="number") source()?.insertNode(parent,child);else renderer.insert(parent,child,0);};
    if(key==="insertNode") return (parent:number,node:number,anchor?:number)=>source()?.insertNode(parent,node,anchor);
    if(key==="setProp") return (node:number,name:string,value:unknown)=>options.setProperty(node,name,value);
    const value=source()?.[key as keyof SceneModule];
    return typeof value==="function" ? value.bind(source()) : value;
  },
});
export const createElement=(...args:unknown[])=>{refresh();return (options.createElement as Function)(...args);};
export const createTextNode=(value:string|number)=>{refresh();return options.createTextNode(value);};
export const insert=(...args:unknown[])=>{refresh();return (renderer.insert as Function)(...args);};
export const insertNode=(parent:number,node:number,anchor?:number)=>source()?.insertNode(parent,node,anchor);
export const setProp=(node:number,name:string,value:unknown)=>{refresh();return options.setProperty(node,name,value);};
export const effect=(...args:unknown[])=>{refresh();return (renderer.effect as Function)(...args);};
export const memo=(...args:unknown[])=>{refresh();return (renderer.memo as Function)(...args);};
export { createComponent } from "solid-js";
