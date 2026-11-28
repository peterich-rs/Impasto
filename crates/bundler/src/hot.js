function start(state, natives) {
  const modules = new Map();
  const components = new Map();
  const records = new WeakMap();
  const kept = new Map();
  const maps = new Map();
  let pending;

  function remember(factory) {
    if (!factory.map) return;
    maps.delete(factory.location);
    maps.set(factory.location, factory.map);
    if (maps.size > 512) maps.delete(maps.keys().next().value);
  }

  function symbolize(stack) {
    return stack.replace(/\b(impasto-hot-[0-9a-f]{16}):(\d+)(?::(\d+))?/g,
      (match, location, line, column) => {
        const map = maps.get(location);
        if (!map) return match;
        map.lines ??= decodeSourceMap(map);
        const records = map.lines[Number(line) - 2];
        if (!records?.length) return match;
        let record = records[0];
        const target = column === undefined ? record[0] : Number(column) - 1;
        for (const candidate of records) {
          if (candidate[0] > target) break;
          record = candidate;
        }
        return record[1] === undefined ? match : `${map.sources[record[1]]}:${record[2] + 1}:${record[3] + 1}`;
      });
  }

  console.trace = (...values) => {
    const stack = new Error().stack?.split("\n").slice(1).join("\n") ?? "";
    console.debug(...values, symbolize(stack));
  };

  function load(id) {
    const staged = pending?.modules;
    if (staged?.has(id)) return staged.get(id).exports;
    if (!pending?.replay.has(id) && modules.has(id)) return modules.get(id).exports;
    const factory = (pending?.next ?? state).factories[id];
    if (!factory) throw new Error(`Unknown development module ${id}`);
    const owner = staged ?? modules;
    const module = { exports: {} };
    owner.set(id, module);
    try {
      execute(factory, id, module);
      return module.exports;
    } catch (error) {
      owner.delete(id);
      throw error;
    }
  }

  function execute(factory, id, module) {
    const require = (name) => {
      if (Object.hasOwn(natives, name)) return natives[name];
      const filename = name.startsWith(".")
        ? new URL(name, `file:///${id}`).pathname.slice(1)
        : name;
      return load(factory.dependencies[name] ?? filename);
    };
    if (pending) pending.maps.push(factory);
    else remember(factory);
    const parent = pending?.executing;
    if (pending) pending.executing = id;
    try {
      compileHotFactory(factory.location, factory.body)(module, module.exports, require);
    } finally {
      if (pending) pending.executing = parent;
    }
  }

  globalThis.__impasto_hot = {
    symbolize,
    keep(module, name, signature, init) {
      const key = JSON.stringify([module, name]);
      const entry = pending?.kept.get(key) ?? kept.get(key);
      if (entry?.signature === signature) return entry.value;
      if (pending && signature === null) {
        throw new Error(`Cannot initialize module state during HMR: ${name}`);
      }
      const value = init();
      (pending?.kept ?? kept).set(key, { signature, value });
      return value;
    },
    component(module, name, signature, implementation) {
      const key = JSON.stringify([module, name]);
      const prior = components.get(key);
      if (pending && prior) {
        if (prior.signature !== signature || pending.replay.get(pending.executing)) {
          pending.signals.set(prior, { signature, implementation });
        }
        return prior.proxy;
      }
      const existing = pending?.components.get(key);
      if (existing) return existing.proxy;
      const [read, write] = createSignal(implementation);
      const proxy = (...args) => untrack(read)(...args);
      const record = { signature, proxy, read, write };
      if (pending) pending.components.set(key, record);
      else components.set(key, record);
      records.set(proxy, record);
      return proxy;
    },
    render(createComponent, component, props) {
      const record = records.get(component);
      if (!record) return createComponent(component, props);
      return createMemo(() => createComponent(record.read(), props));
    },
    patch(patch) {
      if (patch.from !== state.version) return false;
      const factories = { ...state.factories, ...patch.factories };
      for (const id of patch.removed ?? []) delete factories[id];
      const next = { ...state, factories, version: patch.to };
      const staged = {
        next,
        replay: new Map(Object.entries(patch.replay ?? {})),
        modules: new Map(),
        kept: new Map(),
        components: new Map(),
        signals: new Map(),
        maps: [],
      };
      pending = staged;
      try {
        for (const id of staged.replay.keys()) {
          if (modules.has(id)) load(id);
        }
      } catch (error) {
        for (const factory of staged.maps) remember(factory);
        throw error;
      } finally {
        pending = undefined;
      }
      for (const id of patch.removed ?? []) modules.delete(id);
      for (const [id, module] of staged.modules) modules.set(id, module);
      for (const [key, entry] of staged.kept) kept.set(key, entry);
      for (const [key, record] of staged.components) components.set(key, record);
      for (const factory of staged.maps) remember(factory);
      for (const id of Object.keys(patch.factories)) {
        if (!staged.modules.has(id) && modules.has(id)) remember(factories[id]);
      }
      state = next;
      for (const [record, change] of staged.signals) {
        record.signature = change.signature;
        record.write(() => change.implementation);
      }
      return true;
    },

  };
  load(state.entry);
}

// Source map lines decode only when a stack mentions this factory.
function decodeSourceMap(map) {
  const digits = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  let source = 0, line = 0, column = 0;
  return map.mappings.split(";").map((row) => {
    let generated = 0;
    if (!row) return [];
    return row.split(",").map((segment) => {
      const fields = [];
      let value = 0, shift = 0;
      for (const character of segment) {
        const digit = digits.indexOf(character);
        value += (digit & 31) * 2 ** shift;
        if (digit & 32) {
          shift += 5;
        } else {
          fields.push((value & 1 ? -1 : 1) * Math.floor(value / 2));
          value = 0;
          shift = 0;
        }
      }
      generated += fields[0];
      if (fields.length < 4) return [generated];
      source += fields[1];
      line += fields[2];
      column += fields[3];
      return [generated, source, line, column];
    });
  });
}
