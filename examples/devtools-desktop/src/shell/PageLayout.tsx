import { Show } from "solid-js";
import { Column, Row, type UIChildren } from "@impasto/solid-ui";
import { Bar, Divider, Label, Spacer, Tool } from "../components/ui";
import { SplitHandle } from "../components/SplitPane";
import { useWorkspace } from "../state/workspace";
import { useTokens } from "../theme/tokens";

export function PageLayout(props: { title: string; actions?: UIChildren; children?: UIChildren; inspector?: UIChildren; inspectorTitle?: string }) {
  const ws = useWorkspace();
  const c = useTokens();
  return <Row style={{ minHeight: 0, minWidth: 0, flex: 1, overflow: "hidden" }}>
    <Column style={{ flex: 1, minHeight: 0, minWidth: 0, backgroundColor: c().canvas, overflow: "hidden" }}>
      <Bar>
        <Label size={16} weight={600}>{props.title}</Label>
        <Spacer />
        {props.actions}
      </Bar>
      <Divider />
      {props.children}
    </Column>
    <Show when={ws.inspector()}>
      <SplitHandle vertical resize={delta => ws.setInspectorWidth(value => Math.min(500, Math.max(256, value + delta)))} />
      <Column style={{ width: ws.inspectorWidth(), minWidth: 256, minHeight: 0, backgroundColor: c().panel, overflow: "hidden" }}>
        <Bar>
          <Label size={13} weight={600}>{props.inspectorTitle ?? "Inspector"}</Label>
          <Spacer />
          <Tool label="Close inspector" icon="close" iconOnly onClick={() => ws.showInspector(false)} />
        </Bar>
        <Divider />
        {props.inspector}
      </Column>
    </Show>
  </Row>;
}
