import { Show, createTrackedEffect, onCleanup } from "solid-js";
import { Text, type NodeHandle } from "@impasto/solid-renderer";
import { bindScrollStyle, interpolate } from "./scroll-style.ts";
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
      <Show when={props.scrollEffect === "fade-backdrop" && props.scroll}>
        <view
          ref={(node) => {
            if (node === null) return;
            createTrackedEffect(() => {
              const scroll = props.scroll;
              if (scroll) onCleanup(bindScrollStyle(node, scroll, {
                backdropBlur: interpolate([0, 88], [0, 14]),
              }));
            });
          }}
          style={{ position: "absolute", top: 0, left: 0, width: "100%", height: "100%" }}
        >
          <view
            ref={(node) => {
              if (node === null) return;
              createTrackedEffect(() => {
                const scroll = props.scroll;
                if (scroll) onCleanup(bindScrollStyle(node, scroll, {
                  opacity: interpolate([0, 88], [0, 0.86]),
                }));
              });
            }}
            style={{ width: "100%", height: "100%", backgroundColor: theme().color.surface }}
          />
        </view>
      </Show>
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
