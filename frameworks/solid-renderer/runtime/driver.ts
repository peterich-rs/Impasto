import type { ListDriver } from "../src/host.ts";

export type DriverId = { index: number; generation: number };

export type ListOp =
  | { kind: "detach"; key: string }
  | { kind: "adopt"; key: string; type: string; index: number }
  | { kind: "set"; key: string; index: number }
  | { kind: "place"; key: string; before: string };

export type ListChrome = {
  thumbOn: boolean;
  thumbLen: number;
  hThumbOn: boolean;
  hThumbLen: number;
  pinIndex: number;
  pinKey: string;
  extent: number;
};

type ListListener = {
  id: DriverId;
  onOp(op: ListOp): void;
  onChrome(chrome: ListChrome): void;
  onEnd(): void;
};

type RefreshListener = {
  id: DriverId;
  fn(): void;
};

const lists: ListListener[] = [];
const refreshes: RefreshListener[] = [];

function same(id: DriverId, index: number, generation: number): boolean {
  return id.index === index && id.generation === generation;
}

/** Fans the single host callback out to the drivers that asked for it. */
export function deliverDriver(
  kind: number,
  index: number,
  generation: number,
  tag: number,
  i0: number,
  i1: number,
  f0: number,
  _f1: number,
  f2: number,
  _f3: number,
  _f4: number,
  f5: number,
  s0: string,
  s1: string,
): void {
  if (kind === 2 && tag === 5) {
    for (const listener of refreshes) {
      if (same(listener.id, index, generation)) {
        listener.fn();
      }
    }
    return;
  }
  if (kind !== 1) {
    return;
  }
  for (const listener of lists) {
    if (!same(listener.id, index, generation)) {
      continue;
    }
    if (tag === 3) {
      listener.onEnd();
      continue;
    }
    if (tag === 2) {
      listener.onChrome({
        thumbOn: (i0 & 1) !== 0,
        thumbLen: f0,
        hThumbOn: (i0 & 2) !== 0,
        hThumbLen: f2,
        pinIndex: i1,
        pinKey: s0,
        extent: f5,
      });
      continue;
    }
    if (tag !== 1) {
      continue;
    }
    if (i0 === 1) {
      listener.onOp({ kind: "detach", key: s0 });
    } else if (i0 === 2) {
      listener.onOp({ kind: "adopt", key: s0, type: s1, index: i1 });
    } else if (i0 === 3) {
      listener.onOp({ kind: "set", key: s0, index: i1 });
    } else if (i0 === 4) {
      listener.onOp({ kind: "place", key: s0, before: s1 });
    }
  }
}

export function listenList(listener: ListListener): () => void {
  lists.push(listener);
  return () => {
    const index = lists.indexOf(listener);
    if (index >= 0) {
      lists.splice(index, 1);
    }
  };
}

export function listenRefresh(id: DriverId, fn: () => void): () => void {
  const listener = { id, fn };
  refreshes.push(listener);
  return () => {
    const index = refreshes.indexOf(listener);
    if (index >= 0) {
      refreshes.splice(index, 1);
    }
  };
}

export function driverId(driver: { id(): DriverId }): DriverId {
  return driver.id();
}

export type { ListDriver };
