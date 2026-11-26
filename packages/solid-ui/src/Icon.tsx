import { Stack as HostStack } from "@impasto/solid-renderer/host";
import { directionFor, type NodeHandle } from "@impasto/solid-renderer";
import { bindBox, eventsOf } from "./element.tsx";
import type { IconProps } from "./props.ts";
import { useTheme } from "./use-theme.ts";

/** Graphic container. `mirror` flips the horizontal scale in RTL; it does not load an icon font. */
export function Icon(props: IconProps): NodeHandle {
  const theme = useTheme();
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
