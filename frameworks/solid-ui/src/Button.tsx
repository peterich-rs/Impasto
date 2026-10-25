import { createEffect, createSignal } from "solid-js";
import { Text, host, type NodeHandle } from "@uifw/solid-renderer";
import { directionFor } from "./i18n.ts";
import { Role } from "./semantics.ts";
import { minTarget, type ButtonSize, type ButtonVariant } from "./theme.ts";
import { useTheme } from "./use-theme.ts";

export function Button(props: {
  label: string;
  onClick: () => void;
  disabled?: boolean;
  loading?: boolean;
  variant?: ButtonVariant;
  size?: ButtonSize;
  icon?: NodeHandle;
}): NodeHandle {
  const theme = useTheme();
  const [pressed, setPressed] = createSignal(false);
  const blocked = () => !!props.disabled || !!props.loading;
  const size = () => props.size ?? "regular";
  const variant = () => props.variant ?? "quiet";
  let node = 0;

  function fill(): string {
    if (pressed()) {
      return theme().color.textMuted;
    }
    return variant() === "primary" ? theme().color.accent : theme().color.surface;
  }

  function labelColor(): string {
    return variant() === "primary" ? theme().color.bg : theme().color.text;
  }

  function activate(): void {
    if (blocked()) {
      return;
    }
    props.onClick();
  }

  function writeSemantics(): void {
    if (node === 0) {
      return;
    }
    host.setFocusable(node, !blocked());
    host.setSemantics(
      node,
      Role.Button,
      props.label,
      undefined,
      undefined,
      undefined,
      undefined,
      blocked() ? 1 : 0,
    );
    const edge = minTarget(size(), theme().density);
    host.setMinSize(node, edge, edge);
  }

  createEffect(() => {
    writeSemantics();
  });

  return (
    <opacity alpha={blocked() ? 102 : 255}>
      <pointer
        ref={(handle) => {
          node = handle.id;
          writeSemantics();
        }}
        onPress={() => {
          if (!blocked()) {
            setPressed(true);
          }
        }}
        onRelease={() => setPressed(false)}
        onClick={activate}
        onKey={(key) => {
          if (key === "Enter") {
            activate();
          }
        }}
        onText={(value) => {
          if (value === " ") {
            activate();
          }
        }}
      >
        <padding edges={theme().space.s}>
          <decorated color={fill()} radius={theme().radius.s}>
            <row gap={theme().space.s}>
              {props.icon}
              <Text
                fontSize={theme().fontSize.body}
                color={labelColor()}
                weight={theme().weight.emphasis}
                direction={directionFor(theme().locale)}
              >
                {props.label}
              </Text>
            </row>
          </decorated>
        </padding>
      </pointer>
    </opacity>
  );
}
