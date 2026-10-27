import { createTrackedEffect, createSignal } from "solid-js";
import { Text, directionFor, host, type NodeHandle } from "@uifw/solid-renderer";
import { bindFocusRing } from "./focus.ts";
import { Role } from "./semantics.ts";
import { minTarget, type ButtonSize, type ButtonVariant } from "./theme.ts";
import { useTheme } from "./use-theme.ts";

function faded(hex: string, blocked: boolean): string {
  if (!blocked) {
    return hex;
  }
  const body = hex.startsWith("#") ? hex.slice(1) : hex;
  return `#${body.slice(0, 6)}66`;
}

export function Button(props: {
  label: string;
  onClick: () => void;
  disabled?: boolean;
  loading?: boolean;
  variant?: ButtonVariant;
  size?: ButtonSize;
  icon?: NodeHandle;
  /** Span the parent's width. Row siblings share that width. */
  wide?: boolean;
}): NodeHandle {
  const theme = useTheme();
  const [pressed, setPressed] = createSignal(false);
  const [focused, setFocused] = createSignal(false);
  const blocked = () => !!props.disabled || !!props.loading;
  const size = () => props.size ?? "regular";
  const variant = () => props.variant ?? "quiet";
  let node = 0;
  let box = 0;
  bindFocusRing(() => box, focused);

  const scale = () => theme().textScale;
  const padX = () => (size() === "compact" ? theme().space.m : theme().space.l) * scale();
  const padY = () => (size() === "compact" ? theme().space.s : theme().space.l) * scale();
  const target = () => minTarget(size(), theme().density) * scale();

  function face(): string {
    const color = pressed()
      ? variant() === "primary"
        ? theme().color.primaryContainer
        : theme().color.surfaceContainer
      : variant() === "primary"
        ? theme().color.accent
        : theme().color.surface;
    return faded(color, blocked());
  }

  function labelColor(): string {
    const color =
      variant() === "primary"
        ? pressed()
          ? theme().color.onPrimaryContainer
          : theme().color.onPrimary
        : theme().color.text;
    return faded(color, blocked());
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
    const edge = target();
    host.setMinSize(node, edge, edge);
  }

  createTrackedEffect(() => {
    writeSemantics();
  });

  return (
    <pointer
      ref={(handle) => {
        node = handle.id;
        writeSemantics();
      }}
      w={props.wide ? "100%" : undefined}
      onPress={() => {
        if (!blocked()) {
          setPressed(true);
        }
      }}
      onRelease={() => setPressed(false)}
      onClick={activate}
      onFocus={() => setFocused(true)}
      onBlur={() => setFocused(false)}
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
      <decorated
        ref={(handle) => {
          box = handle.id;
        }}
        w={props.wide ? "100%" : undefined}
        color={face()}
        radius={theme().shape.full}
        min={{ h: target() }}
      >
        <padding
          left={padX()}
          right={padX()}
          top={padY()}
          bottom={padY()}
          w={props.wide ? "100%" : undefined}
        >
          <row gap={theme().space.s} justify="center" w={props.wide ? "100%" : undefined}>
            {props.icon}
            <Text
              fontSize={theme().typography.label.size}
              color={labelColor()}
              weight={theme().typography.label.weight}
              direction={directionFor(theme().locale)}
            >
              {props.label}
            </Text>
          </row>
        </padding>
      </decorated>
    </pointer>
  );
}
