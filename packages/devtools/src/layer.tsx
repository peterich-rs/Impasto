/**
 * Devtools top layer: floating button, error banner, and the panel shell.
 * Everything renders with the ordinary solid-ui controls; no second UI stack.
 */
import { createSignal, For, Show } from "solid-js";
import {
  callVoid,
  scene,
  onFrame,
  registerTopLayer,
  type NodeHandle,
} from "@impasto/solid-renderer";
import { ScrollArea, Text, useTheme } from "@impasto/solid-ui";
import { HighlightLayer } from "./highlight.tsx";
import { bannerError, highlights, store, unseenErrors } from "./store.ts";
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
      <stack
        style={{
          translateX: spot().x,
          translateY: spot().y,
        }}
      >
        <view
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
        </view>
        <Show when={unseenErrors() > 0}>
          <stack
            style={{
              translateX: BUTTON_SIZE - 18,
              translateY: -2,
            }}
          >
            <view
              style={{
                backgroundColor: theme().color.danger,
                borderRadius: 9,
              }}
            >
              <view
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
              </view>
            </view>
          </stack>
        </Show>
      </stack>
    </Show>
  );
}
function ErrorBanner(props: { open(): void }) {
  const [message, setMessage] = createSignal<string | null>(null);
  let seenSeq = -1;
  let elapsed = 0;
  onFrame((_target, interval) => {
    const error = bannerError();
    if (error && error.seq !== seenSeq) {
      seenSeq = error.seq;
      elapsed = 0;
      setMessage(error.message.split("\n")[0] ?? error.message);
      return;
    }
    if (message() !== null) {
      elapsed += interval;
      if (elapsed >= 5000) {
        setMessage(null);
      }
    }
  });
  return (
    <Show when={message() !== null}>
      <view
        onClick={() => {
          setMessage(null);
          props.open();
        }}
      >
        <view
          style={{
            backgroundColor: "#c0392b",
          }}
        >
          <view
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
          </view>
        </view>
      </view>
    </Show>
  );
}
function Panel(props: { close(): void }) {
  const theme = useTheme();
  const [page, setPage] = createSignal("console");
  const pages = devtoolPages();
  const current = () => pages.find((entry) => entry.id === page()) ?? pages[0];
  return (
    <stack
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
      <view onClick={() => props.close()}>
        <view
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
      </view>
      <view
        style={{
          padding: 24,
        }}
      >
        <view
          style={{
            backgroundColor: theme().color.background,
            borderRadius: 12,
            flex: 1,
          }}
        >
          <view
            style={{
              width: "100%",
              height: "100%",
              flex: 1,
              flexShrink: 1,
              minWidth: 0,
              minHeight: 0,
            }}
          >
            <view
              style={{
                flexDirection: "row",
                gap: 4,
              }}
            >
              <For each={pages}>
                {(entry: DevtoolPage) => (
                  <view onClick={() => setPage(entry.id)} focusable>
                    <view
                      style={{
                        backgroundColor:
                          page() === entry.id
                            ? theme().color.primary
                            : theme().color.surface,
                        borderRadius: 6,
                      }}
                    >
                      <view
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
                      </view>
                    </view>
                  </view>
                )}
              </For>
            </view>
            <Show when={current()} keyed>
              {(entry) => (
                <ScrollArea axis="vertical">{entry.render()}</ScrollArea>
              )}
            </Show>
          </view>
        </view>
      </view>
    </stack>
  );
}

/** The whole devtools layer: button + banner + panel, mounted on top. */
export function DevtoolsLayer(): NodeHandle {
  store.attach();
  scene.debugSetScan?.(true);
  const root = (
    <stack
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
      <Show when={!store.pageOpen()}>
        <ErrorBanner open={() => store.open()} />
      </Show>
      <Show when={!store.pageOpen()}>
        <DevtoolsButton open={() => store.open()} />
      </Show>
      <Show when={store.pageOpen()}>
        <Panel close={() => store.close()} />
      </Show>
      <HighlightLayer boxes={highlights()} />
    </stack>
  );
  scene.debugSetOwned?.(root, true);
  return root;
}

/** Marks the devtools subtree so probes skip it. */
export function markDevtoolsRoot(id: number): void {
  callVoid("debugSetOwned", [id, true]);
}
