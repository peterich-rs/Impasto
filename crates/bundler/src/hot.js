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
    if (modules.has(id)) return modules.get(id).exports;
    const factory = state.factories[id];
    if (!factory) throw new Error(`Unknown development module ${id}`);
    const module = { exports: {} };
    modules.set(id, module);
    try {
      execute(factory, id, module);
      return module.exports;
    } catch (error) {
      modules.delete(id);
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
    remember(factory);
    compileHotFactory(factory.location, factory.body)(module, module.exports, require);
  }

  globalThis.__impasto_hot = {
    symbolize,
    keep(module, name, init) {
      const key = JSON.stringify([module, name]);
      if (!kept.has(key)) kept.set(key, init());
      return kept.get(key);
    },
    component(module, name, signature, implementation) {
      const key = JSON.stringify([module, name]);
      const prior = components.get(key);
      if (pending) {
        if (!prior) throw new Error(`Unknown mounted component ${name}`);
        if (prior.signature !== signature) pending.set(prior, { signature, implementation });
        return prior.proxy;
      }
      const [read, write] = createSignal(implementation);
      const proxy = (...args) => untrack(read)(...args);
      const record = { signature, proxy, read, write };
      components.set(key, record);
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
      return this.update({
        ...state,
        version: patch.to,
        factories: { ...state.factories, ...patch.factories },
      });
    },
    update(next) {
      const priorIds = Object.keys(state.factories);
      const nextIds = Object.keys(next.factories);
      if (next.entry !== state.entry || priorIds.length !== nextIds.length ||
          JSON.stringify(next.natives) !== JSON.stringify(state.natives)) return false;
      const changed = [];
      for (const id of nextIds) {
        const before = state.factories[id];
        const after = next.factories[id];
        if (!before) return false;
        if (before.body !== after.body) {
          if (id === state.entry || !before.boundary || before.boundary !== after.boundary) return false;
          changed.push(id);
        }
      }
      const staged = new Map();
      pending = staged;
      try {
        for (const id of changed) {
          if (modules.has(id)) execute(next.factories[id], id, { exports: {} });
        }
      } catch (_error) {
        return false;
      } finally {
        pending = undefined;
      }
      for (const [record, change] of staged) {
        record.signature = change.signature;
        record.write(() => change.implementation);
      }
      for (const id of nextIds) {
        if (state.factories[id].body === next.factories[id].body &&
            state.factories[id].map !== next.factories[id].map && modules.has(id)) {
          remember(next.factories[id]);
        }
      }
      state = next;
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
