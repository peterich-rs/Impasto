import { View as HostView } from "@impasto/solid-renderer/host";
import { Text, bindStyle, type NodeHandle } from "@impasto/solid-renderer";
import { StyledText } from "./styled-text.ts";
import { Button } from "./Button.tsx";
import { bindBox } from "./element.tsx";
import { t } from "./i18n.ts";
import type { ErrorPageProps } from "./props.ts";
import { useTheme } from "./use-theme.ts";
import { symbolizeStack } from "./errors.ts";
export {
  installErrorHooks,
  messageOf,
  reportError,
  stackOf,
  useErrorHandler,
} from "./errors.ts";
export function ErrorPage(props: ErrorPageProps): NodeHandle {
  const theme = useTheme();
  return (
    <HostView
      ref={(node) =>
        bindBox(node, props, () => ({
          gap: theme().space.l,
          width: "100%",
        }))
      }
    >
      <StyledText
        class={props.textProps?.class}
        classList={props.textProps?.classList}
        style={props.textProps?.style}
        defaults={{
          fontSize: theme().typography.title.size,
          color: theme().color.danger,
          fontWeight: theme().typography.title.weight,
        }}
      >
        {props.message}
      </StyledText>
      {props.stack ? (
        <Text
          style={{
            fontSize: theme().typography.caption.size,
            color: theme().color.textMuted,
          }}
        >
          {symbolizeStack(props.stack)}
        </Text>
      ) : null}
      <Button
        variant="primary"
        label={t("retry")}
        onClick={() => props.retry()}
        ref={(node) => {
          if (node !== null) bindStyle(node, { defaults: { width: "100%", flexShrink: 1 } });
        }}
        style={props.actionProps?.style}
        class={props.actionProps?.class}
        classList={props.actionProps?.classList}
      />
      {props.onBack ? (
        <Button label={t("back")} onClick={() => props.onBack?.()} />
      ) : null}
    </HostView>
  );
}
