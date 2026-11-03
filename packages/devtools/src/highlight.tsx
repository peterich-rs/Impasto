/**
 * Scene-space outlines drawn over the app. No pointer, so they do not take
 * hits. Each box is four thin bars: a filled rectangle would hide the node.
 */
import { For } from "solid-js";

export type HighlightBox = {
  x: number;
  y: number;
  w: number;
  h: number;
  color: string;
};

const EDGE = 2;

function Outline(props: { box: HighlightBox }) {
  const width = () => Math.max(EDGE, props.box.w);
  const height = () => Math.max(EDGE, props.box.h);
  return (
    <transform x={props.box.x} y={props.box.y}>
      <stack w={width()} h={height()}>
        <decorated color={props.box.color}>
          <column w={width()} h={EDGE} />
        </decorated>
        <transform y={height() - EDGE}>
          <decorated color={props.box.color}>
            <column w={width()} h={EDGE} />
          </decorated>
        </transform>
        <decorated color={props.box.color}>
          <column w={EDGE} h={height()} />
        </decorated>
        <transform x={width() - EDGE}>
          <decorated color={props.box.color}>
            <column w={EDGE} h={height()} />
          </decorated>
        </transform>
      </stack>
    </transform>
  );
}

export function HighlightLayer(props: { boxes: HighlightBox[] }) {
  return (
    <stack fill>
      <For each={props.boxes}>{(box) => <Outline box={box} />}</For>
    </stack>
  );
}
