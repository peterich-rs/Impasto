import { For } from "solid-js";
import { type NodeHandle } from "@uifw/solid-renderer";
import { Button } from "./Button.tsx";
import { Row } from "./Row.tsx";
import { useTheme } from "./use-theme.ts";

/** In-flow tabs: a row of buttons plus the current page. Not an overlay. */
export function Tabs(props: {
  tabs: readonly { id: string; label: string }[];
  selected: string;
  onSelect: (id: string) => void;
  children?: NodeHandle;
}): NodeHandle {
  const theme = useTheme();
  return (
    <column gap={theme().space.s}>
      <Row gap={theme().space.s}>
        <For each={props.tabs}>
          {(tab) => (
            <Button
              label={tab.label}
              variant={tab.id === props.selected ? "primary" : "quiet"}
              onClick={() => props.onSelect(tab.id)}
            />
          )}
        </For>
      </Row>
      <stack>{props.children}</stack>
    </column>
  );
}
