/** Console page: filterable log list. */
import { createSignal, For } from "solid-js";
import { Field, Text, useTheme } from "@impasto/solid-ui";
import { logs, store, type LogLine } from "../store.ts";

const SOURCES = ["", "script", "layout", "host", "net"];

const LEVELS = ["", "ERROR", "WARN", "INFO", "DEBUG", "TRACE"] as const;

export function ConsolePage() {
  const theme = useTheme();
  const [level, setLevel] = createSignal("");
  const [query, setQuery] = createSignal("");
  const [source, setSource] = createSignal("");
  const [target, setTarget] = createSignal("");
  const visible = () =>
    logs().filter(
      (line: LogLine) =>
        (level() === "" || line.level === level()) &&
        (source() === "" || line.source === source()) &&
        (target() === "" || line.target.startsWith(target())) &&
        (query() === "" || line.text.includes(query())),
    );
  return (
    <column fill>
      <row gap={6}>
        <pointer
          onClick={() => {
            const index = LEVELS.indexOf(level() as (typeof LEVELS)[number]);
            setLevel(LEVELS[(index + 1) % LEVELS.length] ?? "");
          }}
        >
          <Text fontSize={11} color={theme().color.textMuted}>
            {level() === "" ? "级别" : level()}
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
      <Field value={target()} placeholder="target" onChange={setTarget} />
      <Field value={query()} placeholder="搜索" onChange={setQuery} />
      <Text fontSize={11} color={theme().color.textMuted}>
        {logs().length} 行
      </Text>
      <column gap={2} grow>
        <For each={visible().slice(-60)}>
          {(line: LogLine) => (
            <decorated
              color={
                line.level === "ERROR"
                  ? theme().color.danger
                  : line.level === "WARN"
                    ? theme().color.warning
                    : theme().color.surface
              }
              radius={4}
            >
              <padding edges={4}>
                <Text fontSize={10} color={theme().color.text}>
                  [{line.level}] {line.target} {line.text}
                </Text>
              </padding>
            </decorated>
          )}
        </For>
      </column>
    </column>
  );
}
