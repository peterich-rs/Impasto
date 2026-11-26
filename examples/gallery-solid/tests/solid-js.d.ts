import "solid-js";

// The Node loader uses the Solid reference runtime, which exposes flush.
declare module "solid-js" {
  export function flush(): void;
}
