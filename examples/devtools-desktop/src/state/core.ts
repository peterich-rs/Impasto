import { createMemo, createSignal, onCleanup } from "solid-js";
import * as core from "@app/devtools-core";

export function streamTicks(onError: (error: unknown) => void) {
  const [tick, setTick] = createSignal<core.TickSummary>();
  const cancel = new AbortController();
  const reader = core.watchTicks(cancel.signal).getReader();
  // The native stream index holds a weak reference. Keep the reader rooted
  // for the lifetime of the workspace, including while read() is pending.
  onCleanup(() => {
    cancel.abort();
    void reader.cancel().catch(() => {});
  });
  async function consume() {
    try {
      while (!cancel.signal.aborted) {
        const update = await reader.read();
        if (update.done) break;
        setTick(update.value);
      }
    } finally {
      reader.releaseLock();
    }
  }
  void consume().catch(error => { if (!cancel.signal.aborted) onError(error); });

  const revision = (key: "framesRev" | "repaintsRev" | "networkRev" | "networkRecordsRev" | "logsRev" | "errorsRev" | "runtimeRev" | "resourcesRev" | "auditsRev" | "sessionRev" | "targetsRev") =>
    createMemo(() => tick()?.[key] ?? 0);
  return {
    tick,
    frame: revision("framesRev"),
    repaint: revision("repaintsRev"),
    network: revision("networkRev"),
    networkRecords: revision("networkRecordsRev"),
    log: revision("logsRev"),
    error: revision("errorsRev"),
    runtime: revision("runtimeRev"),
    resource: revision("resourcesRev"),
    audit: revision("auditsRev"),
    sessionRev: revision("sessionRev"),
    targetsRev: revision("targetsRev"),
  };
}
