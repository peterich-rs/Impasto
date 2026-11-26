import { View as HostView, Stack as HostStack } from "@impasto/solid-renderer/host";
/**
 * Scene-space outlines drawn over the app. No pointer, so they do not take
 * hits. Each box is four thin bars: a filled rectangle would hide the node.
 */
import { For } from "solid-js";
import { scene } from "@impasto/solid-renderer";
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
    <HostStack
      style={{
        translateX: props.box.x,
        translateY: props.box.y,
      }}
    >
      <HostStack
        style={{
          width: width(),
          height: height(),
        }}
      >
        <HostView
          style={{
            backgroundColor: props.box.color,
          }}
        >
          <HostView
            style={{
              width: width(),
              height: EDGE,
            }}
          />
        </HostView>
        <HostStack
          style={{
            translateY: height() - EDGE,
          }}
        >
          <HostView
            style={{
              backgroundColor: props.box.color,
            }}
          >
            <HostView
              style={{
                width: width(),
                height: EDGE,
              }}
            />
          </HostView>
        </HostStack>
        <HostView
          style={{
            backgroundColor: props.box.color,
          }}
        >
          <HostView
            style={{
              width: EDGE,
              height: height(),
            }}
          />
        </HostView>
        <HostStack
          style={{
            translateX: width() - EDGE,
          }}
        >
          <HostView
            style={{
              backgroundColor: props.box.color,
            }}
          >
            <HostView
              style={{
                width: EDGE,
                height: height(),
              }}
            />
          </HostView>
        </HostStack>
      </HostStack>
    </HostStack>
  );
}
export function HighlightLayer(props: { boxes: HighlightBox[] }) {
  return (
    <HostStack
      ref={(node) => {
        if (node === null) return;
        scene.setComponentStyle(node, {
          gridTemplateColumns: "minmax(0, 1fr)",
          gridTemplateRows: "minmax(0, 1fr)",
        });
      }}
      style={{
        width: "100%",
        height: "100%",
        flex: 1,
        flexShrink: 1,
        minWidth: 0,
        minHeight: 0,
      }}
    >
      <For each={props.boxes}>{(box) => <Outline box={box} />}</For>
    </HostStack>
  );
}
