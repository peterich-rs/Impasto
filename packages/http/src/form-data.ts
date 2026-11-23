export interface BinaryEntryLike {
  arrayBuffer(): Promise<ArrayBuffer>;
  readonly type?: string;
  readonly name?: string;
}

export type FormDataEntryValue = string | BinaryEntryLike | Uint8Array;

interface StoredEntry {
  name: string;
  value: FormDataEntryValue;
  filename?: string;
}

const encoder = new TextEncoder();

function escapeQuotes(input: string): string {
  return input.replace(/\r/g, "%0D").replace(/\n/g, "%0A").replace(/"/g, "%22");
}

function concatBytes(parts: Uint8Array[]): Uint8Array {
  let total = 0;
  for (const part of parts) total += part.byteLength;
  const out = new Uint8Array(total);
  let offset = 0;
  for (const part of parts) {
    out.set(part, offset);
    offset += part.byteLength;
  }
  return out;
}

export class FormData {
  readonly #entries: StoredEntry[] = [];

  append(name: string, value: FormDataEntryValue, filename?: string): void {
    this.#entries.push({ name: String(name), value, filename });
  }

  set(name: string, value: FormDataEntryValue, filename?: string): void {
    const key = String(name);
    let inserted = false;
    const next: StoredEntry[] = [];
    for (const entry of this.#entries) {
      if (entry.name === key) {
        if (!inserted) {
          next.push({ name: key, value, filename });
          inserted = true;
        }
      } else {
        next.push(entry);
      }
    }
    if (!inserted) {
      next.push({ name: key, value, filename });
    }
    this.#entries.splice(0, this.#entries.length, ...next);
  }

  delete(name: string): void {
    const key = String(name);
    for (let i = this.#entries.length - 1; i >= 0; i -= 1) {
      if (this.#entries[i].name === key) {
        this.#entries.splice(i, 1);
      }
    }
  }

  get(name: string): FormDataEntryValue | null {
    const key = String(name);
    return this.#entries.find((e) => e.name === key)?.value ?? null;
  }

  getAll(name: string): FormDataEntryValue[] {
    const key = String(name);
    return this.#entries.filter((e) => e.name === key).map((e) => e.value);
  }

  has(name: string): boolean {
    const key = String(name);
    return this.#entries.some((e) => e.name === key);
  }

  *entries(): IterableIterator<[string, FormDataEntryValue]> {
    for (const entry of this.#entries) {
      yield [entry.name, entry.value];
    }
  }

  *keys(): IterableIterator<string> {
    for (const entry of this.#entries) {
      yield entry.name;
    }
  }

  *values(): IterableIterator<FormDataEntryValue> {
    for (const entry of this.#entries) {
      yield entry.value;
    }
  }

  [Symbol.iterator](): IterableIterator<[string, FormDataEntryValue]> {
    return this.entries();
  }

  forEach(
    callback: (value: FormDataEntryValue, key: string, parent: this) => void,
    thisArg?: unknown,
  ): void {
    for (const entry of this.#entries) {
      callback.call(thisArg, entry.value, entry.name, this);
    }
  }

  async serializeMultipart(): Promise<{ contentType: string; body: Uint8Array }> {
    const boundary = `----impastoFormBoundary${Math.random().toString(36).slice(2)}${Date.now().toString(36)}`;
    const chunks: Uint8Array[] = [];

    for (const entry of this.#entries) {
      const dispositionName = escapeQuotes(entry.name);
      if (typeof entry.value === "string") {
        chunks.push(
          encoder.encode(
            `--${boundary}\r\nContent-Disposition: form-data; name="${dispositionName}"\r\n\r\n${entry.value}\r\n`,
          ),
        );
      } else if (entry.value instanceof Uint8Array) {
        const filename = escapeQuotes(entry.filename ?? "blob");
        chunks.push(
          encoder.encode(
            `--${boundary}\r\nContent-Disposition: form-data; name="${dispositionName}"; filename="${filename}"\r\nContent-Type: application/octet-stream\r\n\r\n`,
          ),
          entry.value,
          encoder.encode("\r\n"),
        );
      } else {
        const bytes = new Uint8Array(await entry.value.arrayBuffer());
        const filename = escapeQuotes(entry.filename ?? entry.value.name ?? "blob");
        const mediaType = entry.value.type || "application/octet-stream";
        chunks.push(
          encoder.encode(
            `--${boundary}\r\nContent-Disposition: form-data; name="${dispositionName}"; filename="${filename}"\r\nContent-Type: ${mediaType}\r\n\r\n`,
          ),
          bytes,
          encoder.encode("\r\n"),
        );
      }
    }
    chunks.push(encoder.encode(`--${boundary}--\r\n`));
    return {
      contentType: `multipart/form-data; boundary=${boundary}`,
      body: concatBytes(chunks),
    };
  }
}
