/**
 * Devtools top layer: floating button, error banner, and the panel shell.
 * Everything renders with the ordinary solid-ui controls; no second UI stack.
 */
import { createSignal, For, Show } from "solid-js";
import {
  callVoid,
  host,
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

type ButtonSpot = { x: number; y: number };

function viewport(): { width: number; height: number } {
  let width = 360;
  let height = 640;
  try {
    const metrics = host.metrics();
    if (metrics) {
      width = metrics.width;
      height = metrics.height;
    }
  } catch {
    /* host without metrics keeps the phone default */
  }
  return { width, height };
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
  const x = center < width / 2 ? BUTTON_MARGIN : width - BUTTON_SIZE - BUTTON_MARGIN;
  return clampSpot({ x, y: spot.y });
}

function DevtoolsButton(props: { open(): void }) {
  const theme = useTheme();
  const [spot, setSpot] = createSignal<ButtonSpot>(clampSpot({ x: 1e6, y: 1e6 }));
  const [hidden, setHidden] = createSignal(false);
  return (
    <Show when={!hidden()}>
      <transform x={spot().x} y={spot().y}>
        <pointer
          onClick={() => props.open()}
          recognizers={{ drag: "free", longPress: true }}
          onDragBy={(dx: number, dy: number) =>
            setSpot((prev) => clampSpot({ x: prev.x + dx, y: prev.y + dy }))
          }
          onDragEnd={() => setSpot((prev) => snapSpot(prev))}
          onLongPress={() => setHidden(true)}
        >
          <decorated
            color={theme().color.primary}
            radius={BUTTON_SIZE / 2}
            w={BUTTON_SIZE}
            h={BUTTON_SIZE}
          >
            <column cross="center" justify="center" fill>
              <Text fontSize={22} color={theme().color.onPrimary}>
                ◈
              </Text>
            </column>
          </decorated>
        </pointer>
        <Show when={unseenErrors() > 0}>
          <transform x={BUTTON_SIZE - 18} y={-2}>
            <decorated color={theme().color.danger} radius={9}>
              <column cross="center" justify="center" w={18} h={18}>
                <Text fontSize={10} color="#ffffff">
                  {unseenErrors() > 9 ? "9+" : String(unseenErrors())}
                </Text>
              </column>
            </decorated>
          </transform>
        </Show>
      </transform>
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
      <pointer
        onClick={() => {
          setMessage(null);
          props.open();
        }}
      >
        <decorated color="#c0392b">
          <column cross="center" justify="center" fill h={36}>
            <Text fontSize={12} color="#ffffff" maxLines={1} ellipsis="…">
              {message()}
            </Text>
          </column>
        </decorated>
      </pointer>
    </Show>
  );
}

function Panel(props: { close(): void }) {
  const theme = useTheme();
  const [page, setPage] = createSignal("console");
  const pages = devtoolPages();
  const current = () => pages.find((entry) => entry.id === page()) ?? pages[0];
  return (
    <stack fill>
      <pointer onClick={() => props.close()}>
        <decorated color={theme().color.scrim} fill />
      </pointer>
      <padding edges={24}>
        <decorated color={theme().color.background} radius={12} grow>
          <column fill>
            <row gap={4}>
              <For each={pages}>
                {(entry: DevtoolPage) => (
                  <pointer onClick={() => setPage(entry.id)} focusable>
                    <decorated
                      color={page() === entry.id ? theme().color.primary : theme().color.surface}
                      radius={6}
                    >
                      <padding edges={6}>
                        <Text fontSize={11} color={theme().color.text}>
                          {entry.title}
                        </Text>
                      </padding>
                    </decorated>
                  </pointer>
                )}
              </For>
            </row>
            <Show when={current()} keyed>
              {(entry) => <ScrollArea axis="vertical">{entry.render()}</ScrollArea>}
            </Show>
          </column>
        </decorated>
      </padding>
    </stack>
  );
}

/** The whole devtools layer: button + banner + panel, mounted on top. */
export function DevtoolsLayer(): NodeHandle {
  store.attach();
  host.debugSetScan?.(true);
  const root = (
    <stack fill>
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
  host.debugSetOwned?.(root.id, true);
  return root;
}

/** Marks the devtools subtree so probes skip it. */
export function markDevtoolsRoot(id: number): void {
  callVoid("debugSetOwned", [id, true]);
}
