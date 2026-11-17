/** Console page: filterable log list. */
import { createSignal, For } from "solid-js";
import { Field, Text, useTheme } from "@impasto/solid-ui";
import { logs, store, type LogLine } from "../store.ts";
const SOURCES = ["", "script", "layout", "scene", "net"];
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
          gap: 6,
        }}
      >
        <view
          onClick={() => {
            const index = LEVELS.indexOf(level() as (typeof LEVELS)[number]);
            setLevel(LEVELS[(index + 1) % LEVELS.length] ?? "");
          }}
        >
          <Text
            style={{
              fontSize: 11,
              color: theme().color.textMuted,
            }}
          >
            {level() === "" ? "级别" : level()}
          </Text>
        </view>
        <view
          onClick={() => {
            const index = SOURCES.indexOf(source());
            setSource(SOURCES[(index + 1) % SOURCES.length] ?? "");
          }}
        >
          <Text
            style={{
              fontSize: 11,
              color: theme().color.textMuted,
            }}
          >
            {source() === "" ? "来源" : source()}
          </Text>
        </view>
        <view onClick={() => store.clear()}>
          <Text
            style={{
              fontSize: 11,
              color: theme().color.textMuted,
            }}
          >
            清空
          </Text>
        </view>
      </view>
      <Field value={target()} placeholder="target" onChange={setTarget} />
      <Field value={query()} placeholder="搜索" onChange={setQuery} />
      <Text
        style={{
          fontSize: 11,
          color: theme().color.textMuted,
        }}
      >
        {logs().length} 行
      </Text>
      <view
        style={{
          gap: 2,
          flex: 1,
        }}
      >
        <For each={visible().slice(-60)}>
          {(line: LogLine) => (
            <view
              style={{
                backgroundColor:
                  line.level === "ERROR"
                    ? theme().color.danger
                    : line.level === "WARN"
                      ? theme().color.warning
                      : theme().color.surface,
                borderRadius: 4,
              }}
            >
              <view
                style={{
                  padding: 4,
                }}
              >
                <Text
                  style={{
                    fontSize: 10,
                    color: theme().color.text,
                  }}
                >
                  [{line.level}] {line.target} {line.text}
                </Text>
              </view>
            </view>
          )}
        </For>
      </view>
    </view>
  );
}
