import { For, Show, createSignal, onCleanup } from "solid-js";
import { boot } from "@impasto/solid-renderer";
import { Button, Text, View } from "@impasto/solid-ui";
import { pickFiles } from "@impasto/file-system";
import {
  attach,
  disconnect,
  discover,
  importArtifact,
  preferredTarget,
  snapshot,
  type Snapshot,
  type TargetSummary,
} from "@app/devtools-core";

const panel = "#171b21";
const foreground = "#edf0f4";
const muted = "#a9b4c3";

function Desktop() {
  const [targets, setTargets] = createSignal<TargetSummary[]>([]);
  const [current, setCurrent] = createSignal<Snapshot>();
  const [notice, setNotice] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  let closed = false;
  let polling = false;
  let connection = 0;

  async function refresh() {
    setTargets(await discover());
  }

  async function link(id: string) {
    if (busy()) return;
    connection += 1;
    setBusy(true);
    setNotice("");
    try {
      setCurrent(await attach(id));
    } catch (error) {
      setNotice(String(error));
    } finally {
      setBusy(false);
    }
  }

  async function unlink() {
    connection += 1;
    setBusy(true);
    try {
      await disconnect();
      setCurrent(undefined);
    } catch (error) {
      setNotice(String(error));
    } finally {
      setBusy(false);
    }
  }

  async function openArtifact() {
    try {
      const [file] = await pickFiles({ accept: [".pftrace", ".impasto-session", ".impsym", ".map", ".json"] });
      if (!file) return;
      const artifact = await importArtifact(file.relativePath);
      const count = artifact.packets ?? null;
      const detail = count === null ? `${(artifact.bytes / 1024).toFixed(1)} KiB` : `${count} packets`;
      setNotice(`${artifact.name} · ${artifact.kind} · ${detail}`);
    } catch (error) {
      setNotice(String(error));
    }
  }

  async function update() {
    if (closed || polling || busy() || !current()) return;
    polling = true;
    const expected = connection;
    try {
      const value = await snapshot();
      if (!closed && connection === expected) setCurrent(value ?? undefined);
    } catch (error) {
      if (!closed && connection === expected) {
        setCurrent(undefined);
        setNotice(String(error));
      }
    } finally {
      polling = false;
    }
  }

  const timer = setInterval(() => void update(), 500);
  onCleanup(() => {
    closed = true;
    clearInterval(timer);
  });
  void refresh().catch((error: unknown) => setNotice(String(error)));
  const preferred = preferredTarget();
  if (preferred) void link(preferred);

  return (
    <View style={{ flex: 1, padding: 32, rowGap: 24, backgroundColor: "#101318" }}>
      <View style={{ flexDirection: "row", alignItems: "center", justifyContent: "space-between" }}>
        <Text style={{ color: foreground, fontSize: 28, fontWeight: 700 }}>Impasto DevTools</Text>
        <Button label="Open Artifact…" onClick={() => void openArtifact()} style={{ cursor: "pointer" }} />
      </View>
      <View style={{ flex: 1, flexDirection: "row", columnGap: 20 }}>
        <View style={{ width: 275, padding: 18, rowGap: 10, backgroundColor: panel }}>
          <Text style={{ color: foreground, fontSize: 16, fontWeight: 600 }}>Targets</Text>
          <Button label="Refresh" size="compact" onClick={() => void refresh().catch((error: unknown) => setNotice(String(error)))} />
          <For each={targets()}>
            {(target) => (
              <Button label={`${target.name} · ${target.platform}`} size="compact" disabled={busy()} onClick={() => void link(target.id)} />
            )}
          </For>
          <Show when={!targets().length}>
            <Text style={{ color: muted }}>Run an Impasto app in debug mode to discover it here.</Text>
          </Show>
        </View>
        <View style={{ flex: 1, padding: 22, rowGap: 16, backgroundColor: panel }}>
          <Show when={current()} fallback={<Text style={{ color: muted }}>{busy() ? "Connecting…" : "Select a target to inspect its frame timings."}</Text>}>
            {(target) => (
              <>
                <Text style={{ color: foreground, fontSize: 20, fontWeight: 600 }}>{target().appName}</Text>
                <Text style={{ color: muted, fontFamily: "Monospace" }}>
                  {`${target().platform}   PID ${target().pid}   ${target().width} × ${target().height} pt`}
                </Text>
                <Text style={{ color: foreground, fontFamily: "Monospace" }}>
                  {target().frameNumber
                    ? `Frame ${target().frameNumber}   UI ${target().uiMs?.toFixed(2)} ms   Raster ${target().rasterMs?.toFixed(2)} ms   Budget ${target().budgetMs?.toFixed(2)} ms`
                    : "Waiting for a frame…"}
                </Text>
                <Button label="Disconnect" size="compact" disabled={busy()} onClick={() => void unlink()} />
              </>
            )}
          </Show>
        </View>
      </View>
      <Show when={notice()}>
        <Text style={{ color: muted }}>{notice()}</Text>
      </Show>
    </View>
  );
}

boot(Desktop);
