import { Text, type NodeHandle } from "@impasto/solid-renderer";
import { bindBox, eventsOf } from "./element.tsx";
import type { NavigationBarProps } from "./props.ts";
import { useTheme } from "./use-theme.ts";

/** In-flow title row. Not an overlay. */
export function NavigationBar(props: NavigationBarProps): NodeHandle {
  const theme = useTheme();
  return (
    <view
      {...eventsOf(props)}
      ref={(node) =>
        bindBox(node, props, () => ({
          flexDirection: "row",
          alignItems: "center",
          justifyContent: "space-between",
          gap: theme().space.s,
          width: "100%",
          flex: 1,
          flexShrink: 1,
          minWidth: 0,
          minHeight: 0,
        }))
      }
    >
      {props.leading}
      <Text
        class={props.titleProps?.class}
        classList={props.titleProps?.classList}
        style={props.titleProps?.style}
      >
        {props.title}
      </Text>
      {props.trailing}
    </view>
  );
}
