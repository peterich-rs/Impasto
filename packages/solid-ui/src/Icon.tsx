import { Stack as HostStack } from "@impasto/solid-renderer/host";
import { directionFor, type NodeHandle } from "@impasto/solid-renderer";
import { Canvas } from "./Canvas.tsx";
import { bindBox, eventsOf } from "./element.tsx";
import type { CanvasProps, IconProps } from "./props.ts";
import { useTheme } from "./use-theme.ts";

/** Graphic container. `mirror` flips the horizontal scale in RTL; it does not load an icon font. */
export function Icon(props: IconProps): NodeHandle {
  const theme = useTheme();
  if (props.asset) {
    const canvas: CanvasProps = {
      get children() { return undefined; },
      get style() {
        const size = { width: props.size ?? props.asset?.width, height: props.size ?? props.asset?.height };
        const style = props.style;
        return style && "length" in style ? [size, ...style] : [size, style];
      },
      draw(context, size) {
        const icon = props.asset;
        if (!icon || icon.width <= 0 || icon.height <= 0) return;
        context.save();
        if (props.mirror && directionFor(theme().locale) === "rtl") {
          context.translate(size.width, 0);
          context.scale(-1, 1);
        }
        try {
          context.scale(size.width / icon.width, size.height / icon.height);
          icon.draw(context, { color: props.color });
        } finally {
          context.restore();
        }
      },
    };
    return Canvas(Object.setPrototypeOf(canvas, props));
  }
  return (
    <HostStack
      {...eventsOf(props)}
      ref={(node) =>
        bindBox(node, props, () => ({
          scaleX: directionFor(theme().locale) === "rtl" && props.mirror ? -1 : 1,
        }))
      }
    >
      {props.children}
    </HostStack>
  );
}
