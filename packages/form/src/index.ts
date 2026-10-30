import { createSignal, type Accessor } from "solid-js";

export type FieldErrors<T> = Partial<Record<keyof T, string>>;

export type FormController<T extends Record<string, string>> = {
  values: Accessor<T>;
  errors: Accessor<FieldErrors<T>>;
  setField<K extends keyof T>(key: K, value: T[K]): void;
  handleSubmit(fields: Partial<Record<keyof T, { focus(): void }>>): boolean;
};

function clean<T>(result: Partial<Record<keyof T, string | null | undefined>>): FieldErrors<T> {
  const next: FieldErrors<T> = {};
  for (const key of Object.keys(result) as (keyof T)[]) {
    const value = result[key];
    if (typeof value === "string" && value.length > 0) {
      next[key] = value;
    }
  }
  return next;
}

/**
 * Sync validation runs on every change. Async validation is debounced
 * (`debounceMs`, default 400) and only the last call is applied.
 * `handleSubmit` focuses the first field that has a sync error.
 */
export function createForm<T extends Record<string, string>>(options: {
  initial: T;
  validate: (values: T) => Partial<Record<keyof T, string | null | undefined>>;
  validateAsync?: (values: T) => Promise<Partial<Record<keyof T, string | null | undefined>>>;
  debounceMs?: number;
}): FormController<T> {
  const [values, setValues] = createSignal({ ...options.initial });
  const [errors, setErrors] = createSignal<FieldErrors<T>>({});
  const wait = options.debounceMs ?? 400;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let token = 0;

  function applySync(next: T): FieldErrors<T> {
    return clean(options.validate(next));
  }

  function scheduleAsync(next: T): void {
    if (!options.validateAsync) {
      return;
    }
    if (timer !== undefined) {
      clearTimeout(timer);
    }
    const mine = ++token;
    timer = setTimeout(() => {
      const run = options.validateAsync;
      if (!run) {
        return;
      }
      void run(next).then((result) => {
        if (mine !== token) {
          return;
        }
        setErrors(clean(result));
      });
    }, wait);
  }

  return {
    values,
    errors,
    setField(key, value) {
      const next = { ...values(), [key]: value };
      setValues(next);
      setErrors(applySync(next));
      scheduleAsync(next);
    },
    handleSubmit(fields) {
      const found = applySync(values());
      setErrors(found);
      const first = Object.keys(found)[0] as keyof T | undefined;
      if (first !== undefined) {
        fields[first]?.focus();
        return false;
      }
      return true;
    },
  };
}
