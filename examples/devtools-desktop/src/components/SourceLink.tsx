import { Show } from "solid-js";
import { Row, View } from "@impasto/solid-ui";
import { Tool, Label } from "./ui";
import { useWorkspace } from "../state/workspace";

export function sourceLocation(symbol: string) {
  const match = symbol.trim().match(/(?:\(|\s|^)((?:file:\/\/)?[^()]+?):(\d+):(\d+)\)?$/);
  if (!match) return undefined;
  const path = match[1].trim().replace(/^file:\/\//, "");
  return { path, line: Number(match[2]), column: Number(match[3]) };
}

export function SourceLink(props: { text: string; path?: string; line?: number; column?: number }) {
  const ws = useWorkspace();
  const source = () => props.path ? { path: props.path, line: props.line ?? 1, column: props.column ?? 1 } : sourceLocation(props.text);
  return <Row style={{ gap: 6, minWidth: 0, alignItems: "center" }}>
    <View style={{ flex: 1, minWidth: 0 }}><Label mono size={11} lines={3}>{props.text}</Label></View>
    <Show when={source()}>{location =>
      <Tool label="Open source" icon="open" iconOnly onClick={() => ws.openSource(location().path, location().line, location().column)} />
    }</Show>
  </Row>;
}
