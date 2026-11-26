import { Show, createSignal, onCleanup } from "solid-js";
import { Permission } from "@impasto/permissions";
import { boot } from "@impasto/solid-renderer";
import { Button, Text, View } from "@impasto/solid-ui";
import { CameraPreview, openCamera, type CameraSession } from "@impasto/camera";

function Scanner() {
  const [session, setSession] = createSignal<CameraSession | null>(null);
  const [status, setStatus] = createSignal("Ready to scan");
  let scan: AbortController | undefined;

  function stop() {
    scan?.abort();
    scan = undefined;
    session()?.close();
    setSession(null);
  }

  onCleanup(stop);

  function start() {
    if (scan) return;
    const abort = new AbortController();
    scan = abort;

    void (async () => {
      const permission = await Permission.camera?.request();
      if (abort.signal.aborted) return;
      if (permission !== "granted") {
        setStatus("Camera permission is required");
        return;
      }
      const camera = await openCamera({ facing: "back" });
      if (abort.signal.aborted) {
        camera.close();
        return;
      }
      setSession(camera);
      setStatus("Point the camera at a QR code");
      const reader = camera.watchQrCodes(abort.signal).getReader();
      try {
        while (!abort.signal.aborted) {
          const { value, done } = await reader.read();
          if (done) break;
          if (!abort.signal.aborted) setStatus(value.text);
        }
      } finally {
        reader.releaseLock();
      }
    })().catch((error: unknown) => {
      if (!abort.signal.aborted) setStatus(String(error));
    }).finally(() => {
      if (scan === abort) stop();
    });
  }

  function photo() {
    const camera = session();
    if (!camera) return;
    void camera.capture().then(
      (data) => { if (session() === camera) setStatus(`JPEG · ${data.byteLength} bytes`); },
      (error: unknown) => { if (session() === camera) setStatus(String(error)); },
    );
  }

  return (
    <View style={{ flex: 1, padding: 12, gap: 10 }}>
      <View style={{ flex: 1 }}>
        <Show when={session()}>{(camera) => <CameraPreview session={camera()} />}</Show>
      </View>
      <Text>{status()}</Text>
      <View style={{ flexDirection: "row", gap: 8 }}>
        <Button label="Open" onClick={start} />
        <Button label="Capture" onClick={photo} disabled={!session()} />
        <Button label="Close" onClick={stop} />
      </View>
    </View>
  );
}

boot(Scanner);
