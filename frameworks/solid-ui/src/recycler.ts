/** One live or pooled shell. The component owns the maps; this function only diffs. */
export type RecyclerSlot<T> = {
  key: string;
  type: string;
  node: number;
  set(item: T, index: number): void;
  detach(): void;
  discard(): void;
  place(parent: number, anchor: number): void;
};

export type RecyclerState<T> = {
  live: Map<string, RecyclerSlot<T>>;
  pools: Map<string, RecyclerSlot<T>[]>;
};

export type RecyclerEntry<T> = {
  key: string;
  type: string;
  item: T;
  index: number;
};

export function syncRecycler<T>(options: {
  state: RecyclerState<T>;
  parent: number;
  entries: readonly RecyclerEntry<T>[];
  poolSize: number;
  create(type: string, item: T, index: number): RecyclerSlot<T>;
}): void {
  const { state, parent, entries, poolSize, create } = options;
  const next = new Map<string, RecyclerEntry<T>>();
  const order: string[] = [];
  for (const entry of entries) {
    if (next.has(entry.key)) {
      continue;
    }
    next.set(entry.key, entry);
    order.push(entry.key);
  }
  for (const [key, slot] of state.live) {
    const wanted = next.get(key);
    if (wanted !== undefined && wanted.type === slot.type) {
      continue;
    }
    park(state, slot, poolSize);
    state.live.delete(key);
  }
  const placed: RecyclerSlot<T>[] = [];
  for (const key of order) {
    const wanted = next.get(key);
    if (wanted === undefined) {
      continue;
    }
    let slot = state.live.get(key);
    if (slot === undefined) {
      slot = take(state, wanted, create);
      state.live.set(key, slot);
    } else {
      slot.set(wanted.item, wanted.index);
    }
    placed.push(slot);
  }
  if (parent === 0) {
    return;
  }
  for (let index = placed.length - 1; index >= 0; index -= 1) {
    const anchor = index + 1 < placed.length ? placed[index + 1].node : 0;
    placed[index].place(parent, anchor);
  }
}

function park<T>(state: RecyclerState<T>, slot: RecyclerSlot<T>, poolSize: number): void {
  slot.detach();
  const pooled = state.pools.get(slot.type) ?? [];
  pooled.push(slot);
  state.pools.set(slot.type, pooled);
  while (pooled.length > poolSize) {
    pooled.shift()?.discard();
  }
}

function take<T>(
  state: RecyclerState<T>,
  wanted: RecyclerEntry<T>,
  create: (type: string, item: T, index: number) => RecyclerSlot<T>,
): RecyclerSlot<T> {
  const pooled = state.pools.get(wanted.type);
  const slot = pooled?.pop();
  if (slot === undefined) {
    return create(wanted.type, wanted.item, wanted.index);
  }
  slot.key = wanted.key;
  slot.set(wanted.item, wanted.index);
  return slot;
}

export function discardRecycler<T>(state: RecyclerState<T>): void {
  for (const slot of state.live.values()) {
    slot.discard();
  }
  state.live.clear();
  for (const pooled of state.pools.values()) {
    for (const slot of pooled) {
      slot.discard();
    }
  }
  state.pools.clear();
}
