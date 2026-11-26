import { View as HostView } from "@impasto/solid-renderer/host";
/** Error page: list plus stack detail with sourcemap lines. */
import { createRenderEffect, createSignal, For, Show } from "solid-js";
import { Clipboard } from "@impasto/clipboard";
import { Text, useTheme } from "@impasto/solid-ui";
import { errors, setUnseenErrors } from "../store.ts";
import { SourceMap } from "../source-map.ts";
import * as raw from "@impasto/devtools/native";

/** QuickJS stack frames look like `at fn (file.js:12:34)`. */
const FRAME = /at\s+(?:(\S+)\s+)?\(?(.+?):(\d+):(\d+)\)?/g;
export function ErrorsPage() {
  const theme = useTheme();
  const [selected, setSelected] = createSignal<number>(-1);
  const [map, setMap] = createSignal<SourceMap | null>(null);
  createRenderEffect(
    () => errors().length,
    () => {
      setUnseenErrors(0);
    },
  );
  const current = () => errors().find((entry) => entry.seq === selected());
  const select = (seq: number) => {
    setSelected(seq);
    setUnseenErrors(0);
    if (map() === null) {
      const text = raw.sourceMap?.();
      if (typeof text === "string") {
        setMap(SourceMap.load(text));
      }
    }
  };
  const frames = () => {
    const error = current();
    if (!error?.stack) {
      return [];
    }
    const source = map();
    const out: {
      label: string;
      context: string | null;
    }[] = [];
    for (const match of error.stack.matchAll(FRAME)) {
      const file = match[2] ?? "";
      const line = Number(match[3] ?? "0") - 1;
      const col = Number(match[4] ?? "0") - 1;
      if (source) {
        const position = source.original(line, col);
        if (position) {
          out.push({
            label: `${position.source}:${position.line + 1}:${position.col + 1}`,
            context: source.context(position.source, position.line),
          });
          continue;
        }
      }
      out.push({
        label: `${file}:${line + 1}:${col + 1}`,
        context: null,
      });
    }
    return out;
  };
  return (
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
      <Text
        style={{
          fontSize: 11,
          color: theme().color.textMuted,
        }}
      >
        {errors().length} 条错误
      </Text>
      <HostView
        style={{
          gap: 2,
          flex: 1,
        }}
      >
        <For each={errors().slice(-40)}>
          {(error) => (
            <HostView onClick={() => select(error.seq)}>
              <HostView
                style={{
                  backgroundColor: theme().color.surface,
                  borderRadius: 4,
                }}
              >
                <HostView
                  style={{
                    padding: 4,
                  }}
                >
                  <Text
                    style={{
                      fontSize: 10,
                      color: theme().color.danger,
                      maxLines: 1,
                      ellipsis: "…",
                    }}
                  >
                    [{error.source}] {error.message.split("\n")[0]}
                  </Text>
                </HostView>
              </HostView>
            </HostView>
          )}
        </For>
      </HostView>
      <Show when={current()} keyed>
        {(error) => (
          <HostView
            style={{
              gap: 2,
            }}
          >
            <HostView
              style={{
                flexDirection: "row",
                gap: 8,
              }}
            >
              <Text
                style={{
                  fontSize: 11,
                  color: theme().color.danger,
                }}
              >
                {error.message.split("\n")[0]}
              </Text>
              <HostView
                onClick={() => {
                  Clipboard.setString(error.stack ?? error.message);
                }}
              >
                <Text
                  style={{
                    fontSize: 10,
                    color: theme().color.textMuted,
                  }}
                >
                  复制
                </Text>
              </HostView>
            </HostView>
            <For each={frames()}>
              {(frame) => (
                <HostView
                  style={{
                    gap: 1,
                  }}
                >
                  <Text
                    style={{
                      fontSize: 10,
                      color: theme().color.text,
                    }}
                  >
                    {frame.label}
                  </Text>
                  <Show when={frame.context !== null}>
                    <Text
                      style={{
                        fontSize: 9,
                        color: theme().color.textMuted,
                      }}
                    >
                      {frame.context}
                    </Text>
                  </Show>
                </HostView>
              )}
            </For>
          </HostView>
        )}
      </Show>
    </HostView>
  );
}
