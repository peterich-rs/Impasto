import { Show, createSignal, createTrackedEffect, onCleanup } from "solid-js";
import {
  Text,
  bindListRenderer,
  createSlot,
  directionFor,
  host,
  type NodeHandle,
} from "@impasto/solid-renderer";
import { Button } from "./Button.tsx";
import { ScrollArea, type ScrollPhysicsProps } from "./ScrollArea.tsx";
import {
  createScrollController,
  refreshListener,
  type ScrollController,
} from "./ScrollController.ts";
import { useTheme } from "./use-theme.ts";

const restoredOffsets = new Map<string, { x: number; y: number }>();

type Pin = { index: number; key: string };

/**
 * Mounts only the rows near the viewport. Window, measurement, and chrome
 * live on `ListDriver`. This component is the template and the render callback.
 */
export function VirtualList<T>(props: {
  items: readonly T[];
  itemKey: (item: T) => string | number;
  estimatedHeight: number;
  overscan?: number;
  children?: (item: T, index: number) => NodeHandle;
  recycle?: {
    typeOf: (item: T) => string;
    render: (item: () => T, index: () => number) => NodeHandle;
    poolSize?: number;
  };
  empty?: () => NodeHandle;
  error?: string;
  onRetry?: () => void;
  skeleton?: () => NodeHandle;
  onEndReached?: () => void;
  sticky?: (item: T, index: number) => boolean;
  restoreKey?: string;
  onRefresh?: () => void;
  controller?: ScrollController;
  physics?: ScrollPhysicsProps;
  onDragOverflow?: (overflowX: number, overflowY: number) => boolean;
}): NodeHandle {
  const theme = useTheme();
  const scroll = props.controller ?? createScrollController();
  const native = host.listDriver?.(props.overscan ?? 4, props.estimatedHeight);
  if (native === undefined) {
    throw new Error("listDriver is not linked");
  }
  const initial = props.restoreKey ? restoredOffsets.get(props.restoreKey) : undefined;
  const [extent, setExtent] = createSignal(0);
  const [thumbOn, setThumbOn] = createSignal(false);
  const [thumbH, setThumbH] = createSignal(24);
  const [hThumbOn, setHThumbOn] = createSignal(false);
  const [hThumbW, setHThumbW] = createSignal(24);
  const [pin, setPin] = createSignal<Pin | null>(null);
  let viewport = 0;
  let contentId = 0;
  let top = 0;
  let bottom = 0;
  let rows = 0;
  let vThumb = 0;
  let hThumb = 0;
  let pinNode = 0;
  let vSize = 0;
  let hSize = 0;

  function syncNodes(): void {
    native.bind(viewport, contentId, top, bottom, rows, vThumb, hThumb, pinNode, vSize, hSize);
  }

  createTrackedEffect(() => {
    const items = props.items;
    native.configure(
      props.estimatedHeight,
      props.overscan ?? 4,
      (props.error ?? "").length > 0,
    );
    if (props.onRefresh) {
      scroll.setPull(2, 64);
    }
    native.setItems(
      items.map((item, index) => ({
        key: String(props.itemKey(item)),
        type: props.recycle ? props.recycle.typeOf(item) : "item",
        sticky: props.sticky?.(item, index) === true,
      })),
    );
  });

  onCleanup(
    bindListRenderer(native, {
      parent: () => rows,
      poolSize: () => (props.recycle ? (props.recycle.poolSize ?? 8) : 0),
      itemAt: (index) => props.items[index] as T,
      render: (item, index) => {
        if (props.recycle) {
          return props.recycle.render(item, index);
        }
        return props.children?.(item(), index()) ?? (<sized w={0} h={0} /> as unknown as NodeHandle);
      },
      createSlot,
      onChrome(chrome) {
        setExtent(chrome.extent);
        setThumbOn(chrome.thumbOn);
        setThumbH(chrome.thumbLen);
        setHThumbOn(chrome.hThumbOn);
        setHThumbW(chrome.hThumbLen);
        if (chrome.pinIndex >= 0) {
          setPin({ index: chrome.pinIndex, key: chrome.pinKey });
        } else {
          setPin(null);
        }
      },
      onEnd() {
        if ((props.error ?? "").length > 0) {
          return;
        }
        props.onEndReached?.();
      },
    }),
  );
  onCleanup(refreshListener(scroll, () => props.onRefresh?.()));
  onCleanup(() => {
    const key = props.restoreKey;
    if (key) {
      restoredOffsets.set(key, scroll.offset());
    }
    native.release();
  });

  const errored = () => (props.error ?? "").length > 0;
  const pinned = () => {
    const box = pin();
    if (box === null) {
      return null;
    }
    return props.items[box.index] === undefined ? null : box;
  };

  return (
    <Show
      when={!errored()}
      fallback={
        <column gap={theme().space.s}>
          <Text
            fontSize={theme().fontSize.body}
            color={theme().color.danger}
            direction={directionFor(theme().locale)}
          >
            {props.error ?? ""}
          </Text>
          <Show when={props.onRetry}>
            <Button label="Retry" onClick={() => props.onRetry?.()} />
          </Show>
        </column>
      }
    >
      <Show
        when={props.items.length > 0}
        fallback={
          props.skeleton ? (
            props.skeleton()
          ) : props.empty ? (
            props.empty()
          ) : (
            <sized w={0} h={0} />
          )
        }
      >
        <stack fill>
          <ScrollArea
            axis="vertical"
            contentMin={extent()}
            initialOffset={initial}
            controller={scroll}
            physics={props.physics}
            onDragOverflow={props.onDragOverflow}
            viewportRef={(id) => {
              viewport = id;
              syncNodes();
            }}
            contentRef={(id) => {
              contentId = id;
              syncNodes();
            }}
          >
            <sized
              w={1}
              h={0}
              ref={(node) => {
                top = node.id;
                syncNodes();
              }}
            />
            <column
              ref={(node) => {
                rows = node.id;
                syncNodes();
              }}
            />
            <sized
              w={1}
              h={0}
              ref={(node) => {
                bottom = node.id;
                syncNodes();
              }}
            />
          </ScrollArea>
          <Show when={pinned()} keyed>
            {(box) => (
              <transform
                ref={(node) => {
                  pinNode = node.id;
                  syncNodes();
                }}
              >
                {props.recycle
                  ? props.recycle.render(
                      () => props.items[box.index] as T,
                      () => box.index,
                    )
                  : props.children?.(props.items[box.index] as T, box.index)}
              </transform>
            )}
          </Show>
          <Show when={thumbOn()}>
            <row fill justify="end">
              <padding right={theme().space.s}>
                <transform
                  ref={(node) => {
                    vThumb = node.id;
                    syncNodes();
                  }}
                >
                  <decorated color={theme().color.textMuted} radius={theme().radius.s}>
                    <sized
                      w={3}
                      h={thumbH()}
                      ref={(node) => {
                        vSize = node.id;
                        syncNodes();
                      }}
                    />
                  </decorated>
                </transform>
              </padding>
            </row>
          </Show>
          <Show when={hThumbOn()}>
            <column fill justify="end">
              <padding bottom={theme().space.s}>
                <transform
                  ref={(node) => {
                    hThumb = node.id;
                    syncNodes();
                  }}
                >
                  <decorated color={theme().color.textMuted} radius={theme().radius.s}>
                    <sized
                      w={hThumbW()}
                      h={3}
                      ref={(node) => {
                        hSize = node.id;
                        syncNodes();
                      }}
                    />
                  </decorated>
                </transform>
              </padding>
            </column>
          </Show>
        </stack>
      </Show>
    </Show>
    );
}
