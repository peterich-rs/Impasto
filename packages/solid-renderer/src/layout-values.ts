/** A Taffy size. A bare number is points. `"50%"` is the fraction 0.5. */
export type AxisSize = number | `${number}%` | "auto";

/** One inset edge. `"auto"` clears that edge. A number may be negative. */
export type InsetEdge = number | "auto";

/** One grid line. `line` is a non-zero integer. `span` is an integer >= 1. */
export type GridLine = "auto" | { line: number } | { span: number };

const PERCENT = /^(?:\d+(?:\.\d+)?|\.\d+)%$/;

export function parseAxisSize(value: unknown): AxisSize {
  if (typeof value === "number") {
    if (!Number.isFinite(value) || value < 0) {
      throw new Error("bad axis");
    }
    return value;
  }
  if (value === "auto") {
    return "auto";
  }
  if (typeof value === "string" && PERCENT.test(value)) {
    const number = Number(value.slice(0, -1));
    if (!Number.isFinite(number) || number < 0) {
      throw new Error("bad axis");
    }
    return value as `${number}%`;
  }
  throw new Error("bad axis");
}

export function parseInsetEdge(value: unknown): InsetEdge {
  if (value === "auto") {
    return "auto";
  }
  if (typeof value === "number" && Number.isFinite(value)) {
    return value;
  }
  throw new Error("bad inset");
}

function integerIn(value: unknown, min: number, max: number): number | undefined {
  if (typeof value !== "number" || !Number.isInteger(value) || value < min || value > max) {
    return undefined;
  }
  return value;
}

export function parseGridLine(value: unknown): GridLine {
  if (value === "auto") {
    return "auto";
  }
  if (value !== null && typeof value === "object") {
    const record = value as { line?: unknown; span?: unknown };
    const hasLine = Object.hasOwn(record, "line");
    const hasSpan = Object.hasOwn(record, "span");
    if (hasLine === hasSpan) {
      throw new Error("bad grid line");
    }
    if (hasLine) {
      const line = integerIn(record.line, -32768, 32767);
      if (line === undefined || line === 0) {
        throw new Error("bad grid line");
      }
      return { line };
    }
    const span = integerIn(record.span, 1, 65535);
    if (span === undefined) {
      throw new Error("bad grid line");
    }
    return { span };
  }
  throw new Error("bad grid line");
}
