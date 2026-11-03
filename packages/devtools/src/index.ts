export { installDevtools, type DevtoolsHandle } from "./install.ts";
export { registerDevtoolPage, devtoolPages, type DevtoolPage } from "./registry.ts";
export {
  logs,
  errors,
  network,
  frames,
  unseenErrors,
  store,
  type LogLine,
  type DiagError,
  type NetEntry,
  type FrameSample,
} from "./store.ts";
export { SourceMap } from "./source-map.ts";
