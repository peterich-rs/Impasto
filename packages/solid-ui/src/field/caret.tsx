import { View as HostView, Stack as HostStack } from "@impasto/solid-renderer/host";
import { scene, type NodeHandle } from "@impasto/solid-renderer";
import type { Accessor } from "solid-js";
const HIDDEN = "transparent";

/**
 * Selection highlights plus the 2px caret. The caret color is the accent while
 * focused, collapsed, and blinking; otherwise it is transparent.
 */
export function CaretLayer(props: {
  rects: Accessor<
    {
      x: number;
      y: number;
      w: number;
      h: number;
    }[]
  >;
  caretX: Accessor<number>;
  caretY: Accessor<number>;
  caretH: Accessor<number>;
  showCaret: Accessor<boolean>;
  accent: Accessor<string>;
  transparent: Accessor<string>;
}): NodeHandle {
  return (
    <HostStack>
      {props.rects().map((rect) => (
        <HostStack
          style={{
            translateX: rect.x,
            translateY: rect.y,
          }}
        >
          <HostView
            style={{
              backgroundColor: `${props.accent()}66`,
            }}
          >
            <HostView
              style={{
                width: rect.w,
                height: rect.h,
              }}
            />
          </HostView>
        </HostStack>
      ))}
      <HostStack
        style={{
          translateX: props.caretX(),
          translateY: props.caretY(),
        }}
      >
        <HostView
          ref={(node) => {
            if (node !== null) scene.setRepaintBoundary?.(node, true);
          }}
          style={{
            backgroundColor: props.showCaret()
              ? props.accent()
              : props.transparent() || HIDDEN,
          }}
        >
          <HostView
            style={{
              width: 2,
              height: props.caretH(),
            }}
          />
        </HostView>
      </HostStack>
    </HostStack>
  );
}
