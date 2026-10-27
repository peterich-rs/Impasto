import { type NodeHandle } from "@uifw/solid-renderer";
import type { Accessor } from "solid-js";

const HIDDEN = "transparent";

/**
 * Selection highlights plus the 2px caret. The caret color is the accent while
 * focused, collapsed, and blinking; otherwise it is transparent.
 */
export function CaretLayer(props: {
  rects: Accessor<{ x: number; y: number; w: number; h: number }[]>;
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
        <transform x={rect.x} y={rect.y}>
          <decorated color={`${props.accent()}66`}>
            <sized w={rect.w} h={rect.h} />
          </decorated>
        </transform>
      ))}
      <transform x={props.caretX()} y={props.caretY()}>
        <decorated color={props.showCaret() ? props.accent() : (props.transparent() || HIDDEN)}>
          <sized w={2} h={props.caretH()} />
        </decorated>
      </transform>
    </stack>
  );
}
