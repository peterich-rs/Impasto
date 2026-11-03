/**
 * Minimal source-map consumer: nearest mapping for (line, column). Supports
 * what esbuild emits for the single-file gallery bundle: `sources`,
 * `sourcesContent`, and standard VLQ `mappings`. Lines decode lazily, in
 * order, at most once.
 */

type Segment = {
  /** Column in the generated line, 0-based. */
  col: number;
  /** `-1` when the segment has no source. */
  source: number;
  sourceLine: number;
  sourceCol: number;
  name?: number;
};

const BASE64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const CHAR_TO_VALUE = new Map<string, number>();
for (let index = 0; index < BASE64.length; index += 1) {
  CHAR_TO_VALUE.set(BASE64[index]!, index);
}

/** Returns [value, next index]. Continuation bit 32 chains digits. */
function decodeVlq(text: string, at: number): [number, number] {
  let shift = 0;
  let value = 0;
  let index = at;
  for (;;) {
    const char = text[index];
    if (char === undefined) {
      return [0, index];
    }
    const digit = CHAR_TO_VALUE.get(char);
    if (digit === undefined) {
      return [0, index];
    }
    index += 1;
    value += (digit & 31) << shift;
    if ((digit & 32) === 0) {
      break;
    }
    shift += 5;
  }
  const negative = (value & 1) === 1;
  value >>= 1;
  return [negative ? -value : value, index];
}

export type OriginalPosition = {
  source: string;
  line: number;
  col: number;
  name?: string;
};

export class SourceMap {
  private readonly lines = new Map<number, Segment[]>();
  private decodedThrough = -1;
  readonly sources: string[];
  readonly sourcesContent: (string | null)[];
  private readonly mappings: string;
  private readonly names: string[];

  private constructor(
    sources: string[],
    sourcesContent: (string | null)[],
    mappings: string,
    names: string[],
  ) {
    this.sources = sources;
    this.sourcesContent = sourcesContent;
    this.mappings = mappings;
    this.names = names;
  }

  static load(text: string): SourceMap | null {
    let raw: unknown;
    try {
      raw = JSON.parse(text);
    } catch {
      return null;
    }
    if (!raw || typeof raw !== "object") {
      return null;
    }
    const map = raw as {
      sources?: unknown;
      sourcesContent?: unknown;
      mappings?: unknown;
      names?: unknown;
    };
    if (typeof map.mappings !== "string" || !Array.isArray(map.sources)) {
      return null;
    }
    const sources = map.sources.map((item) => String(item));
    const sourcesContent = Array.isArray(map.sourcesContent)
      ? map.sourcesContent.map((item) => (typeof item === "string" ? item : null))
      : sources.map(() => null);
    const names = Array.isArray(map.names) ? map.names.map((item) => String(item)) : [];
    return new SourceMap(sources, sourcesContent, map.mappings, names);
  }

  /** Line and column are 0-based. Null when the bundle has no mapping. */
  original(line: number, col: number): OriginalPosition | null {
    if (line > this.decodedThrough) {
      this.decodeUpTo(line);
    }
    const segments = this.lines.get(line);
    if (!segments || segments.length === 0) {
      return null;
    }
    let best: Segment | null = null;
    for (const segment of segments) {
      if (segment.col <= col) {
        best = segment;
      } else {
        break;
      }
    }
    const pick = best ?? segments[0]!;
    if (pick.source < 0 || pick.source >= this.sources.length) {
      return null;
    }
    return {
      source: this.sources[pick.source]!,
      line: pick.sourceLine,
      col: pick.sourceCol,
      name: pick.name === undefined ? undefined : this.names[pick.name],
    };
  }

  /** The source line with `radius` lines of context, when bundled. */
  context(source: string, line: number, radius = 2): string | null {
    const index = this.sources.indexOf(source);
    if (index < 0) {
      return null;
    }
    const text = this.sourcesContent[index];
    if (!text) {
      return null;
    }
    const lines = text.split("\n");
    const first = Math.max(0, line - radius);
    const last = Math.min(lines.length - 1, line + radius);
    const out: string[] = [];
    for (let at = first; at <= last; at += 1) {
      const marker = at === line ? ">" : " ";
      out.push(`${marker} ${String(at + 1).padStart(4)} ${lines[at] ?? ""}`);
    }
    return out.join("\n");
  }

  /** Decodes mapping lines forward to `target`, at most once each. */
  private decodeUpTo(target: number): void {
    let source = 0;
    let sourceLine = 0;
    let sourceCol = 0;
    let name = 0;
    let lineNumber = 0;
    const text = this.mappings;
    let index = 0;
    while (index < text.length && lineNumber <= target) {
      const segments: Segment[] = [];
      let col = 0;
      while (index < text.length && text[index] !== ";") {
        let value = 0;
        [value, index] = decodeVlq(text, index);
        col += value;
        const hasSource =
          index < text.length && text[index] !== "," && text[index] !== ";";
        if (hasSource) {
          [value, index] = decodeVlq(text, index);
          source += value;
          [value, index] = decodeVlq(text, index);
          sourceLine += value;
          [value, index] = decodeVlq(text, index);
          sourceCol += value;
          const hasName = index < text.length && text[index] !== "," && text[index] !== ";";
          if (hasName) {
            [value, index] = decodeVlq(text, index);
            name += value;
            segments.push({ col, source, sourceLine, sourceCol, name });
          } else {
            segments.push({ col, source, sourceLine, sourceCol });
          }
        } else {
          segments.push({ col, source: -1, sourceLine: 0, sourceCol: 0 });
        }
        if (text[index] === ",") {
          index += 1;
        }
      }
      this.lines.set(lineNumber, segments);
      lineNumber += 1;
      if (text[index] === ";") {
        index += 1;
      }
    }
    this.decodedThrough = Math.max(this.decodedThrough, lineNumber - 1);
  }
}
