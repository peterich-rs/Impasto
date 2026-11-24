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
    <stack>
      {props.rects().map((rect) => (
        <stack
          style={{
            translateX: rect.x,
            translateY: rect.y,
          }}
        >
          <view
            style={{
              backgroundColor: `${props.accent()}66`,
            }}
          >
            <view
              style={{
                width: rect.w,
                height: rect.h,
              }}
            />
          </view>
        </stack>
      ))}
      <stack
        style={{
          translateX: props.caretX(),
          translateY: props.caretY(),
        }}
      >
        <view
          ref={(node) => {
            if (node !== null) scene.setRepaintBoundary?.(node, true);
          }}
          style={{
            backgroundColor: props.showCaret()
              ? props.accent()
              : props.transparent() || HIDDEN,
          }}
        >
          <view
            style={{
              width: 2,
              height: props.caretH(),
            }}
          />
        </view>
      </stack>
    </stack>
  );
}
