import { For } from "solid-js";
import { type NodeHandle } from "@impasto/solid-renderer";
import { bindBox, eventsOf } from "./element.tsx";
import { Button } from "./Button.tsx";
import { Row } from "./Row.tsx";
import type { TabsProps } from "./props.ts";
import { useTheme } from "./use-theme.ts";

/** In-flow tabs: a row of buttons plus the current page. Not an overlay. */
export function Tabs(props: TabsProps): NodeHandle {
  const theme = useTheme();
  return (
    <view
      {...eventsOf(props)}
      ref={(node) =>
        bindBox(node, props, () => ({
          gap: theme().space.s,
        }))
      }
    >
      <Row style={{ gap: theme().space.s }}>
        <For each={props.tabs}>
          {(tab) => (
            <Button
              label={tab.label}
              variant={tab.id === props.selected ? "primary" : "quiet"}
              onClick={() => props.onSelect(tab.id)}
              class={props.tabProps?.class}
              classList={props.tabProps?.classList}
              style={props.tabProps?.style}
            />
          )}
        </For>
      </Row>
      <stack
        class={props.panelProps?.class}
        classList={props.panelProps?.classList}
        style={props.panelProps?.style}
      >
        {props.children}
      </stack>
    </view>
  );
}
