import { createRenderEffect, createSignal } from "solid-js";
import { Text, host, onFrame, requestFrame, type NodeHandle } from "@uifw/solid-renderer";
import { FONT_SIZE, TEXT_COLOR } from "@uifw/solid-renderer/defaults";

const TEXT_BYTES = [255, 255, 255, 255];

const FIELD_BORDER = "#3A3A3A";
const FIELD_BORDER_FOCUS = "#7CFF6B";
const CURSOR = "#7CFF6B";
const LINE = 22;
const MEASURE_WIDTH = 1_000_000;

function popChar(value: string): string {
  const chars = Array.from(value);
  chars.pop();
  return chars.join("");
}

/** Scroll the nearest scroll ancestor so `id`'s layout box sits inside the viewport. */
function reveal(id: number): void {
  const field = host.rect(id);
  let parent = host.parent(id);
  while (parent !== 0) {
    let offset: { x: number; y: number };
    try {
      offset = host.scrollOffset(parent);
    } catch {
      parent = host.parent(parent);
      continue;
    }
    const view = host.rect(parent);
    let y = offset.y;
    const paintedBottom = field.y - y + field.h;
    if (paintedBottom > view.y + view.h) {
      y += paintedBottom - (view.y + view.h);
    }
    if (field.y - y < view.y) {
      y -= view.y - (field.y - y);
    }
    if (y < 0) {
      y = 0;
    }
    if (y !== offset.y) {
      host.setScrollOffset(parent, offset.x, y);
    }
    parent = host.parent(parent);
  }
}

/**
 * A single-line field. The string, caret, and composition live here.
 * `core` only tracks which node is focused and routes keys and text.
 */
export function Field(props: { placeholder?: string; onInput?: (value: string) => void }): NodeHandle {
  const [text, setText] = createSignal("");
  const [composing, setComposing] = createSignal<string | null>(null);
  const [focused, setFocused] = createSignal(false);
  const [blink, setBlink] = createSignal(true);
  const [caret, setCaret] = createSignal(0);
  let node = 0;

  const body = () => text() + (composing() ?? "");
  const shown = () => {
    const value = body();
    if (value.length === 0) {
      return props.placeholder ?? "";
    }
    return value;
  };

  onFrame((timeMs) => {
    if (!focused()) {
      return;
    }
    setBlink(Math.floor(timeMs / 500) % 2 === 0);
    requestFrame();
  });

  createRenderEffect(body, (value) => {
    const size = host.measureText(value, FONT_SIZE, MEASURE_WIDTH, TEXT_BYTES);
    setCaret(size.w);
  });

  return (
    <pointer
      ref={(handle) => {
        node = handle.id;
      }}
      focusable
      onFocus={() => {
        setFocused(true);
        setBlink(true);
        host.setImeEnabled(true);
        host.setImeText(text());
        reveal(node);
        requestFrame();
      }}
      onBlur={() => {
        setFocused(false);
        host.setImeEnabled(false);
      }}
      onKey={(key) => {
        if (key === "Backspace") {
          const next = popChar(text());
          setText(next);
          host.setImeText(next);
        }
        if (key === "Enter") {
          props.onInput?.(text());
        }
      }}
      onText={(value) => setText(text() + value)}
      onComposing={(value) => setComposing(value)}
    >
      <padding edges={8}>
        <decorated color={focused() ? FIELD_BORDER_FOCUS : FIELD_BORDER} radius={4}>
          <padding edges={8}>
            <stack>
              <Text fontSize={FONT_SIZE} color={TEXT_COLOR} maxLines={1}>
                {shown()}
              </Text>
              <transform x={caret()}>
                <decorated color={focused() && blink() ? CURSOR : "#00000000"}>
                  <sized w={1} h={LINE} />
                </decorated>
              </transform>
            </stack>
          </padding>
        </decorated>
      </padding>
    </pointer>
  );
}
