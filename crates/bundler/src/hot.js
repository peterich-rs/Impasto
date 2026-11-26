function start(state, natives) {
  const modules = new Map();
  const components = new Map();
  const records = new WeakMap();
  const kept = new Map();
  let pending;

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
    new Function("module", "exports", "require", factory.body)(module, module.exports, require);
  }

  globalThis.__impasto_hot = {
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
      state = next;
      return true;
    },
  };
  load(state.entry);
}
