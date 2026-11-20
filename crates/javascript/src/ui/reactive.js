// Only API adapters live here. Storage, dependency edges and scheduling are Rust.
import * as graph from "#reactive";
function createSignal(value) {
  const id = graph.createSignal(value);
  return [() => graph.get(id), next => untrack(() => {
    const value = typeof next === "function" ? next(graph.get(id)) : next;
    graph.set(id, value);
    return value;
  })];
}
function createMemo(compute) {
  const id = graph.createMemo(compute);
  return () => graph.get(id);
}
function createRenderEffect(compute, effect, options) {
  let previous;
  graph.createRenderEffect(compute, value => {
    const cleanup = effect(value, previous);
    previous = value;
    return cleanup;
  }, options?.schedule ? 1 : 0);
}
function createTrackedEffect(compute) { graph.createTrackedEffect(compute); }
function createRoot(init) {
  const root = graph.root();
  const previous = graph.pushOwner(root);
  const reader = graph.pushUntrack();
  try { return init(() => graph.dispose(root)); }
  catch (error) { graph.dispose(root); throw error; }
  finally { graph.popUntrack(reader); graph.pushOwner(previous); }
}
function getOwner() { return graph.owner() || null; }
function runWithOwner(owner, fn) {
  const previous = graph.pushOwner(owner || 0);
  const reader = graph.pushUntrack();
  try { return fn(); }
  finally { graph.popUntrack(reader); graph.pushOwner(previous); }
}
function onCleanup(fn) { graph.onCleanup(fn); }
function untrack(fn) {
  const reader = graph.pushUntrack();
  try { return fn(); } finally { graph.popUntrack(reader); }
}
function batch(fn) {
  graph.pushBatch();
  try { return fn(); } finally { graph.popBatch(); }
}
const contexts = new Map();
function createContext(defaultValue, options) {
  const id = Symbol(options?.name);
  function context(props) {
    return createRoot(() => {
    const owner = getOwner();
    if (owner !== null) {
      let values = contexts.get(owner);
      if (!values) { contexts.set(owner, values = new Map()); onCleanup(() => contexts.delete(owner)); }
      values.set(id, props.value);
    }
    return children(() => props.children);
    });
  }
  context.id = id;
  context.defaultValue = defaultValue;
  context.Provider = context;
  return context;
}
function useContext(context) {
  let owner = getOwner();
  while (owner) {
    const values = contexts.get(owner);
    if (values?.has(context.id)) return values.get(context.id);
    owner = graph.parentOwner(owner);
  }
  return context.defaultValue;
}
function flatten(value, out) {
  while (typeof value === "function" && value.length === 0) value = value();
  if (Array.isArray(value)) { for (const item of value) flatten(item, out); }
  else if (value != null && value !== false && value !== true) out.push(value);
  return out;
}
function children(fn) {
  const memo = createMemo(() => flatten(fn(), []));
  const read = () => { const list = memo(); return list.length === 1 ? list[0] : list; };
  read.toArray = () => memo().slice();
  return read;
}
const errors = new Map();
function catchError(fn, handler) {
  const owner = getOwner();
  if (owner) { errors.set(owner, handler); onCleanup(() => errors.delete(owner)); }
  try { return fn(); } catch (error) { handler(error); }
}
function handleError(error, owner) {
  while (owner) {
    const handler = errors.get(owner);
    if (handler) { handler(error); return true; }
    owner = graph.parentOwner(owner);
  }
  return false;
}
globalThis.__reactive = { createSignal, createMemo, createRenderEffect, createTrackedEffect, createRoot, getOwner, runWithOwner, onCleanup, untrack, batch, createContext, useContext, children, catchError, handleError };
