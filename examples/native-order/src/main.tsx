import { createSignal, onCleanup } from "solid-js";
import { boot } from "@impasto/solid-renderer";
import { Text, View } from "@impasto/solid-ui";
import { openStore, type OrderStore } from "@app/order";

function Orders() {
  const [status, setStatus] = createSignal("Opening orders…");
  const abort = new AbortController();
  let store: OrderStore | undefined;

  onCleanup(() => {
    abort.abort();
    store?.close();
  });

  void (async () => {
    const opened = await openStore();
    if (abort.signal.aborted) {
      opened.close();
      return;
    }
    store = opened;
    try {
      const orders = await opened.query();
      if (abort.signal.aborted) return;

      const reader = opened.watch(abort.signal).getReader();
      try {
        const { value, done } = await reader.read();
        if (!done && !abort.signal.aborted) {
          const message = `${orders.length} order · ${value.id} · ${value.totalCents}¢`;
          setStatus(message);
          console.info(`native-order: ${message}`);
        }
      } finally {
        try {
          await reader.cancel();
        } finally {
          reader.releaseLock();
        }
      }
    } finally {
      opened.close();
      store = undefined;
    }
  })().catch((error: unknown) => {
    if (!abort.signal.aborted) {
      setStatus(String(error));
      console.error(`native-order: ${String(error)}`);
    }
  });

  return (
    <View style={{ flex: 1, paddingTop: 64, paddingLeft: 24, backgroundColor: "#ffffff" }}>
      <Text style={{ color: "#161616", fontSize: 22 }}>{status()}</Text>
    </View>
  );
}

boot(Orders);
