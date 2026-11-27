import { installMock } from "./driver";
import type { ConnectivityState, ConnectivityType } from "./types";

type State = ConnectivityState & { types: ConnectivityType[] };
export function mockConnectivity(initial: ConnectivityState = { types: ["none"], isConnected: false, isExpensive: false, isConstrained: false }) {
  let state: State = { ...initial, types: [...initial.types] };
  const subscribers = new Set<(state: State) => void>();
  const stops = new Set<() => void>();
  const restore = installMock({
    async getStatus() { return { ...state, types: [...state.types] }; },
    watchStatus(signal) {
      let stop = () => {};
      return new ReadableStream<State>({
        start(controller) {
          if (signal?.aborted) { controller.error(signal.reason); return; }
          const emit = (value: State) => controller.enqueue({ ...value, types: [...value.types] });
          const abort = () => { stop(); controller.error(signal?.reason); };
          const close = () => { stop(); controller.close(); };
          stop = () => { subscribers.delete(emit); stops.delete(close); signal?.removeEventListener("abort", abort); };
          subscribers.add(emit);
          stops.add(close);
          signal?.addEventListener("abort", abort, { once: true });
          emit(state);
        },
        cancel() { stop(); },
      });
    },
  });
  return {
    emit(next: ConnectivityState) {
      if (JSON.stringify(state) === JSON.stringify(next)) return;
      state = { ...next, types: [...next.types] };
      for (const subscriber of subscribers) subscriber(state);
    },
    restore() { for (const stop of stops) stop(); restore(); },
  };
}
