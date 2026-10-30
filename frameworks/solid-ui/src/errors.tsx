import { Text, directionFor, type NodeHandle } from "@impasto/solid-renderer";
import { Button } from "./Button.tsx";
import { t } from "./i18n.ts";
import { useTheme } from "./use-theme.ts";

export { installErrorHooks, messageOf, reportError, stackOf, useErrorHandler } from "./errors.ts";

export function ErrorPage(props: {
  message: string;
  stack?: string;
  retry: () => void;
  onBack?: () => void;
}): NodeHandle {
  const theme = useTheme();
  return (
    <column gap={theme().space.l} w="100%">
      <Text
        fontSize={theme().typography.title.size}
        color={theme().color.danger}
        weight={theme().typography.title.weight}
        direction={directionFor(theme().locale)}
      >
        {props.message}
      </Text>
      {props.stack ? (
        <Text
          fontSize={theme().typography.caption.size}
          color={theme().color.textMuted}
          direction={directionFor(theme().locale)}
        >
          {props.stack}
        </Text>
      ) : null}
      <Button wide variant="primary" label={t("retry")} onClick={() => props.retry()} />
      {props.onBack ? <Button label={t("back")} onClick={() => props.onBack?.()} /> : null}
    </column>
  );
}
