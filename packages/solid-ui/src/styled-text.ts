import {
  Text,
  bindStyle,
  type StyleInput,
  type TextProps,
} from "@impasto/solid-renderer";

/** Component typography stays below the author's class and inline sources. */
export function StyledText(props: TextProps & { defaults: StyleInput }) {
  const node = Text(props);
  bindStyle(node, {
    get defaults() { return props.defaults; },
  });
  return node;
}
