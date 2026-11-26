function inspectSurface(baseline, declared) {
  function memberNames(object, implicit) {
    return Object.fromEntries(Reflect.ownKeys(object).filter(key => !implicit.includes(key)).map(key => {
      const descriptor = Object.getOwnPropertyDescriptor(object, key);
      const name = typeof key === "symbol" ? `[${key.description}]` : key;
      return [name, typeof descriptor.value === "function" ? "method" : "property"];
    }));
  }

  const surface = {};
  for (const key of Reflect.ownKeys(globalThis)) {
    // QuickJS already supplies some globals, such as performance, that the
    // runtime replaces. Inspect those when they belong to the public contract.
    if (baseline.includes(key) && !Object.hasOwn(declared, key)) continue;
    const value = globalThis[key];
    const contract = Object.hasOwn(declared, key) && declared[key];
    if (contract.kind === "class" && typeof value === "function" && value.prototype) {
      const parent = Object.getPrototypeOf(value.prototype);
      const parentName = parent === Object.prototype ? null : parent?.constructor?.name;
      let constructible = true;
      if (contract.constructible) {
        Reflect.construct(function () {}, [], value);
      } else {
        try {
          new value();
        } catch (error) {
          if (!(error instanceof TypeError)) throw error;
          constructible = false;
        }
      }
      surface[key] = {
        kind: "class",
        constructible,
        parent: parentName,
        static: memberNames(value, ["name", "length", "prototype"]),
        prototype: memberNames(value.prototype, ["constructor"]),
      };
    } else if (typeof value === "function") {
      surface[key] = { kind: "function" };
    } else if (typeof value === "object" && value !== null) {
      surface[key] = { kind: "object", own: memberNames(value, []) };
    } else {
      surface[key] = { kind: typeof value };
    }
  }
  return surface;
}
