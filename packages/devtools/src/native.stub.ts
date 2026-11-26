// Node stand-in for the native `@impasto/devtools` module. The panel stays off.
import type {
  DiagError,
  Flags,
  FrameSample,
  LogLine,
  LogSource,
  NetEntry,
  Page,
  Platform,
} from "@impasto/devtools/native";

function emptyPage<T>(): Page<T> {
  return { revision: 0, entries: [] };
}
export function enabled(): boolean {
  return false;
}
export function sourceMap(): string | null {
  return null;
}
export function openSource(_path: string, _line: number, _column: number): boolean {
  return false;
}
export function revision(): number {
  return 0;
}
export function logs(_since?: number) {
  return emptyPage<LogLine>();
}
export function errors(_since?: number) {
  return emptyPage<DiagError>();
}
export function network(_since?: number) {
  return emptyPage<NetEntry>();
}
export function frames(_since?: number) {
  return emptyPage<FrameSample>();
}
export function flags(): Flags {
  return {
    layoutScan: false,
    pointerTrace: false,
    suppressDispatch: false,
  };
}
export function setFlags(_flags?: Partial<Flags>): void {}
export function clear(): void {}
export function platform(): Platform {
  return {
    os: "test",
    osVersion: "",
    scale: 1,
    viewport: {
      w: 0,
      h: 0,
      dpr: 1,
    },
    insets: {
      left: 0,
      top: 0,
      right: 0,
      bottom: 0,
    },
    padding: {
      left: 0,
      top: 0,
      right: 0,
      bottom: 0,
    },
    viewInsets: {
      left: 0,
      top: 0,
      right: 0,
      bottom: 0,
    },
    appState: "active",
    log: "info",
    devtools: false,
    dump: null,
  };
}
export function versions() {
  return {
    shell: "",
    wgpu: "",
    parley: "",
    quickjsPin: "",
  };
}
export function logSource(): LogSource[] {
  return ["script", "layout", "host", "net"];
}
