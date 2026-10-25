import { For, Show, createRenderEffect, createSignal, onCleanup } from "solid-js";
import { Text, host, onFrame, requestFrame, type NodeHandle } from "@uifw/solid-renderer";
import { Button } from "./Button.tsx";
import { directionFor } from "./i18n.ts";
import {
  MIN_THUMB,
  PrefixHeights,
  REFRESH_THRESHOLD,
  estimateHeight,
  nearEnd,
  retainHeights,
  stickyTranslate,
  thumbLength,
  thumbOffset,
  windowRange,
} from "./list-window.ts";
import { ScrollArea } from "./ScrollArea.tsx";
import { useTheme } from "./use-theme.ts";

const HEIGHT_EPS = 0.5;

const restoredOffsets = new Map<string, { x: number; y: number }>();

type Measure = { id: number; index: number };
type Pin = { index: number; key: string };

/**
 * Mounts only the rows near the viewport.
 * Heights live in a Fenwick tree; the window is rebuilt from that prefix.
 */
export function VirtualList<T>(props: {
  items: readonly T[];
  itemKey: (item: T) => string | number;
  estimatedHeight: number;
  overscan?: number;
  children: (item: T, index: number) => NodeHandle;
  empty?: () => NodeHandle;
  error?: string;
  onRetry?: () => void;
  skeleton?: () => NodeHandle;
  onEndReached?: () => void;
  sticky?: (item: T, index: number) => boolean;
  restoreKey?: string;
  onRefresh?: () => void;
}): NodeHandle {
  const theme = useTheme();
  const prefix = new PrefixHeights();
  const cache = new Map<string, number>();
  const mounted = new Map<string, Measure>();
  let keySeq = "";
  let viewport = 0;
  let measuring = false;
  let stopMeasure = () => {};
  let pull = 0;
  let endLength = -1;
  let pinIndex = -1;
  let pinKey = "";
  const initial = props.restoreKey ? restoredOffsets.get(props.restoreKey) : undefined;
  const [range, setRange] = createSignal({ start: 0, end: 0, top: 0, bottom: 0 });
  const [thumb, setThumb] = createSignal({ h: MIN_THUMB, y: 0, visible: false });
  const [hThumb, setHThumb] = createSignal({ w: MIN_THUMB, x: 0, visible: false });
  const [pin, setPin] = createSignal<Pin | null>(null);
  const [pinY, setPinY] = createSignal(0);

  function keysOf(items: readonly T[]): string[] {
    return items.map((item) => String(props.itemKey(item)));
  }

  function sequence(keys: readonly string[]): string {
    return keys.join("\0");
  }

  function syncPrefix(items: readonly T[]): void {
    const keys = keysOf(items);
    const next = sequence(keys);
    if (next === keySeq && prefix.length === items.length) {
      return;
    }
    keySeq = next;
    const estimate = estimateHeight(props.estimatedHeight);
    prefix.rebuild(keys.map((key) => cache.get(key) ?? estimate));
    retainHeights(cache, keys);
  }

  function contentWidth(): number {
    if (viewport === 0) {
      return 0;
    }
    const body = host.firstChild(viewport);
    if (body === 0) {
      return 0;
    }
    return host.rect(body).w;
  }

  function updateSticky(offsetY: number): void {
    const sticky = props.sticky;
    if (!sticky || props.items.length === 0) {
      pinIndex = -1;
      pinKey = "";
      setPin(null);
      setPinY(0);
      return;
    }
    let current = -1;
    for (let index = 0; index < props.items.length; index += 1) {
      const item = props.items[index];
      if (item === undefined || !sticky(item, index)) {
        continue;
      }
      if (prefix.itemStart(index) <= offsetY) {
        current = index;
      } else {
        break;
      }
    }
    if (current < 0) {
      pinIndex = -1;
      pinKey = "";
      setPin(null);
      setPinY(0);
      return;
    }
    let nextStart: number | null = null;
    for (let index = current + 1; index < props.items.length; index += 1) {
      const item = props.items[index];
      if (item !== undefined && sticky(item, index)) {
        nextStart = prefix.itemStart(index);
        break;
      }
    }
    const key = String(props.itemKey(props.items[current] as T));
    if (pinIndex !== current || pinKey !== key) {
      pinIndex = current;
      pinKey = key;
      setPin({ index: current, key });
    }
    setPinY(stickyTranslate(nextStart, offsetY, prefix.heightAt(current)));
  }

  function updateChrome(offsetX: number, offsetY: number, viewWidth: number, viewHeight: number): void {
    const content = prefix.total();
    const overflow = content > viewHeight + HEIGHT_EPS;
    if (!overflow || viewHeight <= 0 || content <= 0) {
      setThumb({ h: MIN_THUMB, y: 0, visible: false });
    } else {
      setThumb({
        h: thumbLength(viewHeight, content),
        y: thumbOffset(viewHeight, content, offsetY),
        visible: true,
      });
    }
    const width = contentWidth();
    if (width > viewWidth + HEIGHT_EPS && viewWidth > 0) {
      setHThumb({
        w: thumbLength(viewWidth, width),
        x: thumbOffset(viewWidth, width, offsetX),
        visible: true,
      });
    } else {
      setHThumb({ w: MIN_THUMB, x: 0, visible: false });
    }
    updateSticky(offsetY);
  }

  function maybeEnd(end: number): void {
    if (!props.onEndReached || (props.error ?? "").length > 0) {
      return;
    }
    const length = props.items.length;
    const overscan = props.overscan ?? 4;
    if (!nearEnd(end, length, overscan) || endLength === length) {
      return;
    }
    endLength = length;
    props.onEndReached();
  }

  function place(offsetX: number, offsetY: number, viewWidth: number, viewHeight: number): void {
    syncPrefix(props.items);
    const next = windowRange(prefix, offsetY, viewHeight, props.overscan ?? 4);
    const current = range();
    if (
      current.start !== next.start ||
      current.end !== next.end ||
      current.top !== next.top ||
      current.bottom !== next.bottom
    ) {
      setRange(next);
    }
    updateChrome(offsetX, offsetY, viewWidth, viewHeight);
    maybeEnd(next.end);
  }

  function windowNeedsMeasure(): boolean {
    const windowed = range();
    for (let index = windowed.start; index < windowed.end; index += 1) {
      const item = props.items[index];
      if (item === undefined) {
        continue;
      }
      if (!cache.has(String(props.itemKey(item)))) {
        return true;
      }
    }
    return false;
  }

  function measureMounted(): boolean {
    let pending = false;
    let changed = false;
    for (const [key, entry] of mounted) {
      const box = host.rect(entry.id);
      if (box.h === 0) {
        pending = true;
        continue;
      }
      const prev = prefix.heightAt(entry.index);
      cache.set(key, box.h);
      if (Math.abs(box.h - prev) > HEIGHT_EPS) {
        prefix.add(entry.index, box.h - prev);
        changed = true;
      }
    }
    if (changed && viewport !== 0) {
      const view = host.rect(viewport);
      const offset = host.scrollOffset(viewport);
      place(offset.x, offset.y, view.w, view.h);
    }
    return pending;
  }

  function startMeasure(): void {
    if (measuring) {
      requestFrame();
      return;
    }
    measuring = true;
    stopMeasure();
    stopMeasure = onFrame(() => {
      const pending = measureMounted();
      if (pending || windowNeedsMeasure()) {
        requestFrame();
        return;
      }
      stopMeasure();
      measuring = false;
    });
    requestFrame();
  }

  onCleanup(() => {
    stopMeasure();
  });

  createRenderEffect(() => {
    const blocked = (props.error ?? "").length > 0 || props.items.length === 0;
    syncPrefix(props.items);
    if (blocked) {
      stopMeasure();
      measuring = false;
      updateSticky(0);
      return;
    }
    if (viewport !== 0) {
      const view = host.rect(viewport);
      const offset = host.scrollOffset(viewport);
      place(offset.x, offset.y, view.w, view.h);
    } else {
      place(0, 0, 0, 0);
    }
    if (windowNeedsMeasure()) {
      startMeasure();
    }
  });

  const windowItems = () => props.items.slice(range().start, range().end);
  const errored = () => (props.error ?? "").length > 0;

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
            initialOffset={initial}
            viewportRef={(id) => {
              viewport = id;
              const view = host.rect(id);
              const offset = host.scrollOffset(id);
              place(offset.x, offset.y, view.w, view.h);
              if (windowNeedsMeasure()) {
                startMeasure();
              }
            }}
            onOffset={(offset, id) => {
              const key = props.restoreKey;
              if (key) {
                restoredOffsets.set(key, offset);
              }
              const view = host.rect(id);
              place(offset.x, offset.y, view.w, view.h);
              if (windowNeedsMeasure()) {
                startMeasure();
              }
            }}
            onPull={(delta, overflowing) => {
              if (!props.onRefresh || !overflowing) {
                pull = 0;
                return;
              }
              if (delta > 0) {
                pull += delta;
              }
            }}
            onDragEnd={() => {
              if (props.onRefresh && pull > REFRESH_THRESHOLD) {
                props.onRefresh();
              }
              pull = 0;
            }}
          >
            <Show when={range().top > 0}>
              <sized w={1} h={range().top} />
            </Show>
            <For each={windowItems()} keyed={(item) => props.itemKey(item)}>
              {(item, index) => {
                const abs = () => range().start + index();
                let wrap = 0;
                createRenderEffect(() => {
                  if (wrap !== 0) {
                    mounted.set(String(props.itemKey(item())), { id: wrap, index: abs() });
                  }
                });
                onCleanup(() => {
                  mounted.delete(String(props.itemKey(item())));
                });
                return (
                  <stack
                    ref={(node) => {
                      wrap = node.id;
                      mounted.set(String(props.itemKey(item())), { id: wrap, index: abs() });
                    }}
                  >
                    {props.children(item(), abs())}
                  </stack>
                );
              }}
            </For>
            <Show when={range().bottom > 0}>
              <sized w={1} h={range().bottom} />
            </Show>
          </ScrollArea>
          <Show when={pin()} keyed>
            {(box) => (
              <transform y={pinY()}>
                {props.children(props.items[box.index] as T, box.index)}
              </transform>
            )}
          </Show>
          <Show when={thumb().visible}>
            <row fill justify="end">
              <padding right={theme().space.s}>
                <transform y={thumb().y}>
                  <decorated color={theme().color.textMuted} radius={theme().radius.s}>
                    <sized w={3} h={thumb().h} />
                  </decorated>
                </transform>
              </padding>
            </row>
          </Show>
          <Show when={hThumb().visible}>
            <column fill justify="end">
              <padding bottom={theme().space.s}>
                <transform x={hThumb().x}>
                  <decorated color={theme().color.textMuted} radius={theme().radius.s}>
                    <sized w={hThumb().w} h={3} />
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
