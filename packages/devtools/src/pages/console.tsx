/** Console page: filterable log list. */
import { createSignal, For } from "solid-js";
import { Field, Text, useTheme } from "@impasto/solid-ui";
import { logs, store, type LogLine } from "../store.ts";

const SOURCES = ["", "script", "layout", "host", "net"];

const LEVEL_LABEL = new Map<number, string>([
  [0, "log"],
  [1, "info"],
  [2, "warn"],
  [3, "error"],
  [4, "debug"],
]);

export function ConsolePage() {
  const theme = useTheme();
  const [level, setLevel] = createSignal<number>(-1);
  const [query, setQuery] = createSignal("");
  const [source, setSource] = createSignal("");
  const visible = () =>
    logs().filter(
      (line: LogLine) =>
        (level() < 0 || line.level === level()) &&
        (source() === "" || line.source === source()) &&
        (query() === "" || line.text.includes(query())),
    );
  return (
    <column fill>
      <row gap={6}>
        <pointer onClick={() => setLevel(-1)}>
          <Text fontSize={11} color={theme().color.textMuted}>
            全部
          </Text>
        </pointer>
        <pointer onClick={() => setLevel(3)}>
          <Text fontSize={11} color={theme().color.danger}>
            error
          </Text>
        </pointer>
        <pointer onClick={() => setLevel(2)}>
          <Text fontSize={11} color={theme().color.warning}>
            warn
          </Text>
        </pointer>
        <pointer onClick={() => setLevel(4)}>
          <Text fontSize={11} color={theme().color.textMuted}>
            debug
          </Text>
        </pointer>
        <pointer
          onClick={() => {
            const index = SOURCES.indexOf(source());
            setSource(SOURCES[(index + 1) % SOURCES.length] ?? "");
          }}
        >
          <Text fontSize={11} color={theme().color.textMuted}>
            {source() === "" ? "来源" : source()}
          </Text>
        </pointer>
        <pointer onClick={() => store.clear()}>
          <Text fontSize={11} color={theme().color.textMuted}>
            清空
          </Text>
        </pointer>
      </row>
      <Field value={query()} placeholder="搜索" onChange={setQuery} />
      <Text fontSize={11} color={theme().color.textMuted}>
        {logs().length} 行
      </Text>
      <column gap={2} grow>
        <For each={visible().slice(-60)}>
          {(line: LogLine) => (
            <decorated
              color={
                line.level === 3
                  ? theme().color.danger
                  : line.level === 2
                    ? theme().color.warning
                    : theme().color.surface
              }
              radius={4}
            >
              <padding edges={4}>
                <Text fontSize={10} color={theme().color.text}>
                  [{LEVEL_LABEL.get(line.level) ?? "?"}] {line.source} {line.text}
                </Text>
              </padding>
            </decorated>
          )}
        </For>
      </column>
    </column>
  );
}
