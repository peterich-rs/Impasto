import { installMock } from "./driver";
import type { LocationServiceStatus, Position } from "./types";

function feed<T>() {
  const entries = new Map<ReadableStreamDefaultController<T>, () => void>();
  return {
    stream(signal?: AbortSignal, initial?: T) {
      let finish = () => {};
      return new ReadableStream<T>({
        start(controller) {
          if (signal?.aborted) { controller.error(signal.reason); return; }
          const abort = () => { finish(); controller.error(signal?.reason); };
          finish = () => { entries.delete(controller); signal?.removeEventListener("abort", abort); };
          signal?.addEventListener("abort", abort, { once: true });
          entries.set(controller, finish);
          if (initial !== undefined) controller.enqueue(initial);
        },
        cancel() { finish(); },
      });
    },
    emit(value: T) { for (const controller of entries.keys()) controller.enqueue(value); },
    close() { for (const [controller, finish] of entries) { finish(); controller.close(); } },
  };
}
export function mockGeolocator({ initialPosition, serviceEnabled = true }: { initialPosition?: Position; serviceEnabled?: boolean } = {}) {
  let position = initialPosition;
  let next = 0;
  let service: LocationServiceStatus = serviceEnabled ? "enabled" : "disabled";
  const locations = feed<Position>();
  const states = feed<LocationServiceStatus>();
  const restore = installMock({
    nextRequestId: () => ++next,
    getCurrentPosition: async () => {
      if (service === "disabled") throw new Error("[geolocator:service-disabled] Location services are off");
      if (!position) throw new Error("[geolocator:position-unavailable] No mock location");
      return position;
    },
    cancel: async () => {},
    getLastKnownPosition: async () => position ?? null,
    watchPosition: (_options, signal) => locations.stream(signal, position),
    isLocationServiceEnabled: async () => service === "enabled",
    watchServiceStatus: signal => states.stream(signal, service),
    checkPermission: async () => "granted",
    requestPermission: async () => "granted",
  });
  return {
    emitPosition(value: Position) { position = value; locations.emit(value); },
    emitServiceStatus(value: LocationServiceStatus) { service = value; states.emit(value); },
    restore() { locations.close(); states.close(); restore(); },
  };
}
