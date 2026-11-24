import { Show, createSignal, createTrackedEffect, onCleanup, untrack } from "solid-js";
import {
  Text,
  bindListRenderer,
  bindNodeRef,
  bindStyle,
  createSlot,
  directionFor,
  scene,
  type NodeHandle,
} from "@impasto/solid-renderer";
import { Button } from "./Button.tsx";
import { RefreshControl } from "./RefreshControl.tsx";
import { ScrollArea, type ScrollPhysics } from "./ScrollArea.tsx";
import {
  createScrollController,
  refreshListener,
  type ScrollController,
} from "./ScrollController.ts";
import { useScrollThreshold } from "./scroll-rules.ts";
import type { VirtualListProps } from "./props.ts";
import { useTheme } from "./use-theme.ts";
const restoredOffsets = new Map<
  string,
  {
    x: number;
    y: number;
  }
>();
type Pin = {
  index: number;
  key: string;
};

/**
 * Mounts only the rows near the viewport. Window, measurement, and chrome
 * live on `ListDriver`. This component is the template and the render callback.
 */
export function VirtualList<T>(props: VirtualListProps<T>): NodeHandle {
  const theme = useTheme();
  const scroll = props.controller ?? createScrollController();
  const native = scene.listDriver?.(props.overscan ?? 4, props.estimatedHeight);
  if (native === undefined) {
    throw new Error("listDriver is not linked");
  }
  const initial = props.restoreKey
    ? restoredOffsets.get(props.restoreKey)
    : undefined;
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
  let dragBy = () => {};
  let dragEnd = () => {};
  function syncNodes(): void {
    native.bind(
      viewport,
      contentId,
      top,
      bottom,
      rows,
      vThumb,
      hThumb,
      pinNode,
      vSize,
      hSize,
    );
  }
  createTrackedEffect(() => {
    const items = props.items;
    native.configure(
      props.estimatedHeight,
      props.overscan ?? 4,
      (props.error ?? "").length > 0,
    );
    if (props.refresh) {
      scroll.setPull(0, props.refresh.threshold ?? 64);
    } else if (props.onRefresh) {
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
        return (
          props.children?.(item(), index()) ??
          (
            <view
              style={{
                width: 0,
                height: 0,
              }}
            />
          )
        );
      },
      createSlot,
      onChrome(chrome) {
        setExtent(chrome.extent);
        setThumbOn(chrome.thumbOn);
        setThumbH(chrome.thumbLen);
        setHThumbOn(chrome.hThumbOn);
        setHThumbW(chrome.hThumbLen);
        if (chrome.pinIndex >= 0) {
          setPin({
            index: chrome.pinIndex,
            key: chrome.pinKey,
          });
          props.onPin?.(chrome.pinKey);
        } else {
          setPin(null);
          props.onPin?.(null);
        }
      },

    }),
  );
  onCleanup(
    props.refresh
      ? () => {}
      : refreshListener(scroll, () => props.onRefresh?.()),
  );
  onCleanup(() => {
    const key = props.restoreKey;
    if (key) {
      restoredOffsets.set(key, scroll.offset());
    }
    native.release();
  });
  const nearEnd = useScrollThreshold(scroll, {
    from: "end",
    get value() {
      return props.endReachedDistance ?? (props.overscan ?? 4) * props.estimatedHeight;
    },
    get enabled() {
      return props.onEndReached !== undefined && props.items.length > 0 && !props.error;
    },
  });
  let endLength = 0;
  createTrackedEffect(() => {
    if (!nearEnd()) return;
    untrack(() => {
      const length = props.items.length;
      if (length === endLength) return;
      endLength = length;
      props.onEndReached?.();
    });
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
        <view
          style={{
            gap: theme().space.s,
          }}
        >
          <Text
            style={{
              fontSize: theme().fontSize.body,
              color: theme().color.danger,
              writingDirection: directionFor(theme().locale),
            }}
          >
            {props.error ?? ""}
          </Text>
          <Show when={props.onRetry}>
            <Button label="Retry" onClick={() => props.onRetry?.()} />
          </Show>
        </view>
      }
    >
      <Show
        when={props.items.length > 0}
        fallback={
          props.skeleton ? (
            <view>{props.skeleton()}</view>
          ) : props.empty ? (
            <view>{props.empty()}</view>
          ) : (
            <view
              style={{
                width: 0,
                height: 0,
              }}
            />
          )
        }
      >
        <stack
          ref={(node) => {
            if (node === null) return;
            bindStyle(node, {
              defaults: {
                width: "100%",
                height: "100%",
                flex: 1,
                flexShrink: 1,
                minWidth: 0,
                minHeight: 0,
                overflow: "hidden",
                gridTemplateColumns: "minmax(0, 1fr)",
                gridTemplateRows: "minmax(0, 1fr)",
              },
              get class() {
                return props.class;
              },
              get classList() {
                return props.classList;
              },
              get style() {
                return props.style;
              },
            });
            bindNodeRef(node, () => props.ref);
          }}
        >
          <ScrollArea
            axis="vertical"
            contentMin={extent()}
            initialOffset={initial}
            controller={scroll}
            physics={props.physics}
            class={props.viewportProps?.class}
            classList={props.viewportProps?.classList}
            style={props.viewportProps?.style}
            onDragBy={() => dragBy()}
            onDragEnd={() => dragEnd()}
            viewportRef={(id) => {
              viewport = id ?? 0;
              syncNodes();
            }}
            contentRef={(id) => {
              contentId = id ?? 0;
              syncNodes();
            }}
          >
            {props.refresh ? (
              <RefreshControl
                controller={scroll}
                threshold={props.refresh.threshold}
                onRefresh={() => props.refresh?.onRefresh()}
                gesture={(handlers) => {
                  dragBy = handlers.onDragBy;
                  dragEnd = handlers.onDragEnd;
                }}
              >
                {props.refresh.indicator}
              </RefreshControl>
            ) : null}
            <view
              ref={(node) => {
                if (node === null) return;
                top = node;
                syncNodes();
              }}
              style={{
                width: 1,
                height: 0,
              }}
            />
            <view
              ref={(node) => {
                if (node === null) return;
                rows = node;
                scene.setStyleSurface(node, "list-content");
                bindStyle(node, {
                  get class() {
                    return props.contentProps?.class;
                  },
                  get classList() {
                    return props.contentProps?.classList;
                  },
                  get style() {
                    return props.contentProps?.style;
                  },
                });
                syncNodes();
              }}
            />
            <view
              ref={(node) => {
                if (node === null) return;
                bottom = node;
                syncNodes();
              }}
              style={{
                width: 1,
                height: 0,
              }}
            />
          </ScrollArea>
          <Show when={pinned()} keyed>
            {(box) => (
              <view
                style={{
                  position: "absolute",
                  top: 0,
                  left: 0,
                  width: "100%",
                  alignItems: "stretch",
                }}
                ref={(node) => {
                  if (node === null) return;
                  pinNode = node;
                  syncNodes();
                }}
              >
                {props.recycle
                  ? props.recycle.render(
                      () => props.items[box.index] as T,
                      () => box.index,
                    )
                  : props.children?.(props.items[box.index] as T, box.index)}
              </view>
            )}
          </Show>
          <Show when={thumbOn()}>
            <view
              style={{
                width: "100%",
                height: "100%",
                flex: 1,
                flexShrink: 1,
                minWidth: 0,
                minHeight: 0,
                flexDirection: "row",
                justifyContent: "end",
              }}
            >
              <view
                style={{
                  paddingRight: theme().space.s,
                }}
              >
                <stack
                  ref={(node) => {
                    if (node === null) return;
                    vThumb = node;
                    syncNodes();
                  }}
                >
                  <view
                    style={{
                      backgroundColor: theme().color.textMuted,
                      borderRadius: theme().radius.s,
                    }}
                  >
                    <view
                      ref={(node) => {
                        if (node === null) return;
                        vSize = node;
                        syncNodes();
                      }}
                      style={{
                        width: 3,
                        height: thumbH(),
                      }}
                    />
                  </view>
                </stack>
              </view>
            </view>
          </Show>
          <Show when={hThumbOn()}>
            <view
              style={{
                width: "100%",
                height: "100%",
                flex: 1,
                flexShrink: 1,
                minWidth: 0,
                minHeight: 0,
                justifyContent: "end",
              }}
            >
              <view
                style={{
                  paddingBottom: theme().space.s,
                }}
              >
                <stack
                  ref={(node) => {
                    if (node === null) return;
                    hThumb = node;
                    syncNodes();
                  }}
                >
                  <view
                    style={{
                      backgroundColor: theme().color.textMuted,
                      borderRadius: theme().radius.s,
                    }}
                  >
                    <view
                      ref={(node) => {
                        if (node === null) return;
                        hSize = node;
                        syncNodes();
                      }}
                      style={{
                        width: hThumbW(),
                        height: 3,
                      }}
                    />
                  </view>
                </stack>
              </view>
            </view>
          </Show>
        </stack>
      </Show>
    </Show>
  );
}
