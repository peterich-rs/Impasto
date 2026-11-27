import { installMock } from "./driver";
import type { OpenUrlOptions, UrlListener } from "./types";

export function mockLinking({ initialUrl = null, canOpen = true }: {
  initialUrl?: string | null; canOpen?: boolean | ((url: string) => boolean);
} = {}) {
  const openedUrls: { url: string; options?: OpenUrlOptions }[] = [];
  const listeners = new Set<UrlListener>();
  const closers = new Set<() => void>();
  const canLaunch = (url: string) => typeof canOpen === "function" ? canOpen(url) : canOpen;
  const restore = installMock({
    getInitialURL: () => initialUrl,
    async canOpenURL(url) { return canLaunch(url); },
    async openURL(url, options) {
      openedUrls.push({ url, options: options == null ? undefined : {
        mode: options.mode ?? undefined, toolbarColor: options.toolbarColor ?? undefined,
        dismissButtonStyle: options.dismissButtonStyle ?? undefined,
      } });
      return canLaunch(url);
    },
    addEventListener(_event, listener) { listeners.add(listener); },
    removeEventListener(_event, listener) { listeners.delete(listener); },
    watchUrls(signal) {
      let stop = () => {};
      return new ReadableStream<string>({
        start(controller) {
          if (signal?.aborted) { controller.error(signal.reason); return; }
          const receive: UrlListener = ({ url }) => controller.enqueue(url);
          const abort = () => { stop(); controller.error(signal?.reason); };
          const close = () => { stop(); controller.close(); };
          stop = () => { listeners.delete(receive); closers.delete(close); signal?.removeEventListener("abort", abort); };
          signal?.addEventListener("abort", abort, { once: true });
          listeners.add(receive);
          closers.add(close);
        },
        cancel() { stop(); },
      });
    },
  });
  return {
    openedUrls: openedUrls as readonly { url: string; options?: OpenUrlOptions }[],
    emitUrl(url: string) { for (const listener of listeners) listener({ url }); },
    restore() { for (const close of closers) close(); listeners.clear(); restore(); },
  };
}
