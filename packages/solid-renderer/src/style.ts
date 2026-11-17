export type {
  AbsoluteLength,
  BasisValue,
  BoxStyle,
  Color,
  EffectColor,
  ShadowStyle,
  BoxShadowStyle,
  GradientStyle,
  FlexValue,
  GridLine,
  GridTrack,
  Inherited,
  InsetValue,
  Length,
  MarginValue,
  MaxSize,
  MinSize,
  PaintStyle,
  SizeValue,
  StyleInput,
  TextStyle,
  TrackBound,
  ViewStyle,
} from "./style.gen.ts";

import type { StyleInput } from "./style.gen.ts";

/** Author props shared by view, text, and image. */
export interface StyleProps {
  style?: StyleInput;
  class?: string;
  classList?: Readonly<Record<string, boolean | undefined>>;
}
