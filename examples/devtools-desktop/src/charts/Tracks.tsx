import { For, createMemo, createSignal } from "solid-js";
import { Column, Row, View } from "@impasto/solid-ui";
import type { ProjectedCanvas } from "@app/devtools-core";
import { InteractiveCanvas } from "./InteractiveCanvas";
import { Divider, Label, Spacer } from "../components/ui";
import { useTokens } from "../theme/tokens";

export type Track = { label: string; legend: readonly [string, number][]; unit: string };

export function Legend(props: { entries: readonly [string, number][] }) {
  const c = useTokens();
  return <Row style={{ gap: 12, flexWrap: "wrap" }}>
    <For each={props.entries}>{([label, color]) => <Row style={{ gap: 5, alignItems: "center" }}>
      <View style={{ width: 7, height: 7, borderRadius: 1, flexShrink: 0, backgroundColor: c().buckets[color] }} />
      <Label size={11} subtle>{label}</Label>
    </Row>}</For>
  </Row>;
}

export function Tracks(props: {
  tracks: readonly Track[];
  height?: number;
  onClick?: (time: number, track: number) => void;
} & (
  { project: (width: number, height: number) => ProjectedCanvas[]; projectTrack?: never } |
  { project?: never; projectTrack: (track: number, width: number, height: number) => ProjectedCanvas | undefined }
)) {
  const [width, setWidth] = createSignal(0);
  const [cursor, setCursor] = createSignal<number>();
  const height = () => props.height ?? 63;
  const plots = createMemo(() => width() && props.project ? props.project(width(), height()) : []);
  return <Column style={{ width: "100%" }}>
    <For each={props.tracks}>{(track, index) => {
      const plot = createMemo(() => {
        if (!props.projectTrack) return plots()[index()];
        const w = width();
        return w > 0 ? props.projectTrack(index(), w, height()) : undefined;
      });
      return <Column style={{ gap: 6, marginBottom: 7 }}>
        <Row style={{ alignItems: "center", gap: 9 }}>
          <Label weight={600}>{track.label}</Label>
          <Legend entries={track.legend} />
          <Spacer />
          <Label subtle mono size={11}>{plot() ? `${plot()!.maxY.toFixed(1)} ${track.unit}` : "—"}</Label>
        </Row>
        <InteractiveCanvas label={track.label} height={height()} project={plot}
          onSize={size => { if (index() === 0) setWidth(size.width); }}
          cursor={cursor()}
          onHover={point => setCursor(Math.max(0, Math.min(1, point.x / Math.max(1, point.width))))}
          onLeave={() => setCursor(undefined)}
          onClick={point => {
            const data = plot();
            if (data && point.width > 0) props.onClick?.(data.minX + point.x / point.width * (data.maxX - data.minX), index());
          }}
        />
        <Divider />
      </Column>;
    }}</For>
  </Column>;
}
