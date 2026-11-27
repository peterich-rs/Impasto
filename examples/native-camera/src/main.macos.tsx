import { createSignal } from "solid-js";
import { Permission, openSystemSettings } from "@impasto/permissions";
import { boot } from "@impasto/solid-renderer";
import { Button, Text, View } from "@impasto/solid-ui";

function CameraPermission() {
  const [status, setStatus] = createSignal("Checking camera access…");

  async function update(read: () => Promise<string>) {
    try {
      setStatus(`Camera access · ${await read()}`);
    } catch (error) {
      setStatus(String(error));
    }
  }

  void update(() => Permission.camera.status);

  return (
    <View style={{ flex: 1, padding: 24, gap: 12 }}>
      <Text>{status()}</Text>
      <Button label="Request camera access" onClick={() => void update(() => Permission.camera.request())} />
      <Button label="Open Camera privacy settings" onClick={() => void openSystemSettings(Permission.camera)} />
      <Button label="Refresh camera access" onClick={() => void update(() => Permission.camera.status)} />
    </View>
  );
}

boot(CameraPermission);
