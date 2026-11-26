import { View as HostView, Stack as HostStack } from "@impasto/solid-renderer/host";
/**
 * Devtools top layer: floating button, error banner, and the panel shell.
 * Everything renders with the ordinary solid-ui controls; no second UI stack.
 */
import { createMemo, createSignal, createTrackedEffect, For, onCleanup, Show } from "solid-js";
import {
  callVoid,
  scene,
  registerTopLayer,
  type NodeHandle,
} from "@impasto/solid-renderer";
import { ScrollArea, Text, useTheme } from "@impasto/solid-ui";
import { HighlightLayer } from "./highlight.tsx";
import { bannerError, highlights, store, traceRecording, unseenErrors } from "./store.ts";
import { devtoolPages, type DevtoolPage } from "./registry.ts";
const BUTTON_SIZE = 56;
const BUTTON_MARGIN = 12;
type ButtonSpot = {
  x: number;
  y: number;
};
function viewport(): {
  width: number;
  height: number;
} {
  let width = 360;
  let height = 640;
  try {
    const metrics = scene.metrics();
    if (metrics && metrics.width > 0 && metrics.height > 0) {
      width = metrics.width;
      height = metrics.height;
    }
  } catch {
    /* scene without metrics keeps the phone default */
  }
  return {
    width,
    height,
  };
}
function clampSpot(spot: ButtonSpot): ButtonSpot {
  const { width, height } = viewport();
  const maxX = Math.max(BUTTON_MARGIN, width - BUTTON_SIZE - BUTTON_MARGIN);
  const maxY = Math.max(BUTTON_MARGIN, height - BUTTON_SIZE - BUTTON_MARGIN);
  return {
    x: Math.min(Math.max(spot.x, BUTTON_MARGIN), maxX),
    y: Math.min(Math.max(spot.y, BUTTON_MARGIN), maxY),
  };
}

/** Release sticks the button to the nearer vertical edge. */
function snapSpot(spot: ButtonSpot): ButtonSpot {
  const { width } = viewport();
  const center = spot.x + BUTTON_SIZE / 2;
  const x =
    center < width / 2 ? BUTTON_MARGIN : width - BUTTON_SIZE - BUTTON_MARGIN;
  return clampSpot({
    x,
    y: spot.y,
  });
}
function DevtoolsButton(props: { open(): void }) {
  const theme = useTheme();
  const [spot, setSpot] = createSignal<ButtonSpot>(
    clampSpot({
      x: 1e6,
      y: 1e6,
    }),
  );
  const [hidden, setHidden] = createSignal(false);
  return (
    <Show when={!hidden()}>
      <HostStack
        style={{
          translateX: spot().x,
          translateY: spot().y,
        }}
      >
        <HostView
          style={{
            backgroundColor: theme().color.primary,
            borderRadius: BUTTON_SIZE / 2,
            width: BUTTON_SIZE,
            height: BUTTON_SIZE,
            alignItems: "center",
            justifyContent: "center",
          }}
          onClick={() => props.open()}
          recognizers={{
            drag: "free",
            longPress: true,
          }}
          onDragBy={(dx: number, dy: number) =>
            setSpot((prev) =>
              clampSpot({
                x: prev.x + dx,
                y: prev.y + dy,
              }),
            )
          }
          onDragEnd={() => setSpot((prev) => snapSpot(prev))}
          onLongPress={() => setHidden(true)}
        >
          <Text
            style={{
              fontSize: 22,
              color: theme().color.onPrimary,
            }}
          >
            ◈
          </Text>
        </HostView>
        <Show when={unseenErrors() > 0}>
          <HostStack
            style={{
              translateX: BUTTON_SIZE - 18,
              translateY: -2,
            }}
          >
            <HostView
              style={{
                backgroundColor: theme().color.danger,
                borderRadius: 9,
              }}
            >
              <HostView
                style={{
                  alignItems: "center",
                  justifyContent: "center",
                  width: 18,
                  height: 18,
                }}
              >
                <Text
                  style={{
                    fontSize: 10,
                    color: "#ffffff",
                  }}
                >
                  {unseenErrors() > 9 ? "9+" : String(unseenErrors())}
                </Text>
              </HostView>
            </HostView>
          </HostStack>
        </Show>
      </HostStack>
    </Show>
  );
}
function ErrorBanner(props: { open(): void }) {
  const [message, setMessage] = createSignal<string | null>(null);
  const latest = createMemo(bannerError);
  createTrackedEffect(() => {
    const error = latest();
    setMessage(error ? error.message.split("\n")[0] ?? error.message : null);
    if (!error) return;
    const timer = setTimeout(() => setMessage(null), 5000);
    onCleanup(() => clearTimeout(timer));
  });
  return (
    <Show when={message() !== null}>
      <HostView
        onClick={() => {
          setMessage(null);
          props.open();
        }}
      >
        <HostView
          style={{
            backgroundColor: "#c0392b",
          }}
        >
          <HostView
            style={{
              width: "100%",
              flex: 1,
              flexShrink: 1,
              minWidth: 0,
              minHeight: 0,
              alignItems: "center",
              justifyContent: "center",
              height: 36,
            }}
          >
            <Text
              style={{
                fontSize: 12,
                color: "#ffffff",
                maxLines: 1,
                ellipsis: "…",
              }}
            >
              {message()}
            </Text>
          </HostView>
        </HostView>
      </HostView>
    </Show>
  );
}
function Panel(props: { close(): void; page: () => string; selectPage(page: string): void }) {
  const theme = useTheme();
  const page = props.page;
  const pages = devtoolPages();
  const current = () => pages.find((entry) => entry.id === page()) ?? pages[0];
  return (
    <HostStack
      ref={(node) => {
        if (node === null) return;
        scene.setComponentStyle(node, {
          gridTemplateColumns: "minmax(0, 1fr)",
          gridTemplateRows: "minmax(0, 1fr)",
        });
      }}
      style={{
        width: "100%",
        height: "100%",
        flex: 1,
        flexShrink: 1,
        minWidth: 0,
        minHeight: 0,
      }}
    >
      <HostView onClick={() => props.close()}>
        <HostView
          style={{
            width: "100%",
            height: "100%",
            flex: 1,
            flexShrink: 1,
            minWidth: 0,
            minHeight: 0,
            backgroundColor: theme().color.scrim,
          }}
        />
      </HostView>
      <HostView
        style={{
          padding: 24,
        }}
      >
        <HostView
          style={{
            backgroundColor: theme().color.background,
            borderRadius: 12,
            flex: 1,
          }}
        >
          <HostView
            style={{
              width: "100%",
              height: "100%",
              flex: 1,
              flexShrink: 1,
              minWidth: 0,
              minHeight: 0,
            }}
          >
            <HostView
              style={{
                flexDirection: "row",
                gap: 4,
              }}
            >
              <For each={pages}>
                {(entry: DevtoolPage) => (
                  <HostView onClick={() => props.selectPage(entry.id)} focusable>
                    <HostView
                      style={{
                        backgroundColor:
                          page() === entry.id
                            ? theme().color.primary
                            : theme().color.surface,
                        borderRadius: 6,
                      }}
                    >
                      <HostView
                        style={{
                          padding: 6,
                        }}
                      >
                        <Text
                          style={{
                            fontSize: 11,
                            color: theme().color.text,
                          }}
                        >
                          {entry.title}
                        </Text>
                      </HostView>
                    </HostView>
                  </HostView>
                )}
              </For>
            </HostView>
            <Show when={current()} keyed>
              {(entry) => (
                <ScrollArea axis="vertical">{entry.render()}</ScrollArea>
              )}
            </Show>
          </HostView>
        </HostView>
      </HostView>
    </HostStack>
  );
}

/** The whole devtools layer: button + banner + panel, mounted on top. */
export function DevtoolsLayer(props: { launcher?: boolean } = {}): NodeHandle {
  store.attach();
  const [page, setPage] = createSignal("console");
  const root = (
    <HostStack
      ref={(node) => {
        if (node === null) return;
        scene.setComponentStyle(node, {
          gridTemplateColumns: "minmax(0, 1fr)",
          gridTemplateRows: "minmax(0, 1fr)",
        });
      }}
      style={{
        width: "100%",
        height: "100%",
        flex: 1,
        flexShrink: 1,
        minWidth: 0,
        minHeight: 0,
      }}
    >
      <Show when={!traceRecording() && !store.pageOpen()}>
        <ErrorBanner open={() => store.open()} />
      </Show>
      <Show when={!traceRecording() && props.launcher !== false && !store.pageOpen()}>
        <DevtoolsButton open={() => store.open()} />
      </Show>
      <Show when={!traceRecording() && store.pageOpen()}>
        <Panel close={() => { store.close(); setPage("console"); }} page={page} selectPage={setPage} />
      </Show>
      <Show when={!traceRecording()}>
        <HighlightLayer boxes={highlights()} />
      </Show>
    </HostStack>
  );
  scene.debugSetOwned?.(root, true);
  return root;
}

/** Marks the devtools subtree so probes skip it. */
export function markDevtoolsRoot(id: number): void {
  callVoid("debugSetOwned", [id, true]);
}
