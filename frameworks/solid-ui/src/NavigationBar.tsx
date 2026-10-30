import { Text, directionFor, type NodeHandle } from "@impasto/solid-renderer";
import { Row } from "./Row.tsx";
import { useTheme } from "./use-theme.ts";

/** In-flow title row. Not an overlay. */
export function NavigationBar(props: {
  title: string;
  leading?: NodeHandle;
  trailing?: NodeHandle;
}): NodeHandle {
  const theme = useTheme();
  return (
    <Row gap={theme().space.s} justify="space-between" fill>
      {props.leading}
      <Text
        fontSize={theme().fontSize.title}
        color={theme().color.text}
        weight={theme().weight.emphasis}
        direction={directionFor(theme().locale)}
      >
        {props.title}
      </Text>
      {props.trailing}
    </Row>
  );
}
