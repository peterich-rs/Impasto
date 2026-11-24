import type { PullState } from "./RefreshControl.tsx";
import type { AutofillHint } from "@impasto/solid-renderer";
import type {
  CanvasError,
  CanvasRenderingContext2D,
  CanvasSize,
  ImageProps,
  NodeRef,
  StyleProps,
  TextProps,
  UIChildren,
  ViewEvents,
} from "@impasto/solid-renderer";
import type { InputMask } from "./field/mask.ts";
import type { ActionName, KeyboardName } from "./field/keyboard.ts";
import type { FieldController } from "./field/controller.ts";
import type { Role } from "./semantics.ts";
import type { ScrollPhysics } from "./ScrollArea.tsx";
import type { ScrollController } from "./ScrollController.ts";
import type { ButtonSize, ButtonVariant } from "./theme.ts";

export type { ImageProps, TextProps, NodeRef, UIChildren, ViewEvents };
export type { CanvasError, CanvasRenderingContext2D, CanvasSize };

export interface CanvasController {
  invalidate(): void;
  toLocal(point: { x: number; y: number }): { x: number; y: number } | null;
}

export type CanvasProps = Omit<ViewProps, "children"> & {
  children?: never;
  /** When enabled at mount, draw with the latest dependencies on re-entering the viewport. */
  deferWhenOffscreen?: boolean;
  draw: (context: CanvasRenderingContext2D, size: CanvasSize) => unknown;
  controller?: (controller: CanvasController | null) => void;
  onDrawError?: (error: CanvasError) => void;
};

export interface ElementProps extends StyleProps {
  ref?: NodeRef;
}

export interface ViewProps extends ElementProps, ViewEvents {
  children?: UIChildren;
  focusable?: boolean;
  role?: Role;
  accessibilityLabel?: string;
  accessibilityHint?: string;
}

export type RowProps = ViewProps;
export type ColumnProps = ViewProps;
export type GridProps = ViewProps;
export type StackProps = ViewProps;

export interface PressableProps extends ViewProps {
  disabled?: boolean;
}

export type ButtonProps = Omit<
  ViewProps,
  "children" | "role" | "focusable" | "onClick"
> & {
  label: string;
  onClick: () => void;
  disabled?: boolean;
  loading?: boolean;
  variant?: ButtonVariant;
  size?: ButtonSize;
  icon?: UIChildren;
  labelProps?: StyleProps;
};

export interface IconProps extends ViewProps {
  mirror?: boolean;
}

export interface SwitchProps extends ElementProps {
  checked?: boolean;
  onChange?: (checked: boolean) => void;
  disabled?: boolean;
  onLabel?: string;
  offLabel?: string;
  label?: string;
  trackProps?: StyleProps;
  labelProps?: StyleProps;
}

export interface FieldProps extends ElementProps {
  autofill?: AutofillHint;
  value?: string;
  placeholder?: string;
  onChange?: (value: string) => void;
  onSubmit?: (value: string) => void;
  multiline?: boolean;
  keyboard?: KeyboardName;
  action?: ActionName;
  maxLines?: number;
  maxLength?: number;
  disabled?: boolean;
  readOnly?: boolean;
  error?: string;
  validate?: (value: string) => string | null | undefined;
  label?: string;
  rows?: number;
  mask?: InputMask;
  transform?: (value: string) => string;
  controller?: (controller: FieldController) => void;
  controlProps?: StyleProps;
  textProps?: StyleProps;
  labelProps?: StyleProps;
  errorProps?: StyleProps;
  inputRef?: NodeRef;
}

export type TextAreaProps = Omit<FieldProps, "multiline">;

export interface NavigationBarProps extends ViewProps {
  scroll?: ScrollController;
  scrollEffect?: "fade-backdrop";
  title: string;
  leading?: UIChildren;
  trailing?: UIChildren;
  titleProps?: StyleProps;
}

export interface TabItem {
  id: string;
  label: string;
}

export interface TabsProps extends ViewProps {
  /** Horizontal ScrollArea whose pages follow the same order as tabs. */
  pager?: ScrollController;
  tabs: readonly TabItem[];
  selected: string;
  onSelect: (id: string) => void;
  titleProps?: StyleProps;
  tabProps?: StyleProps;
  panelProps?: StyleProps;
}

export interface ScrollAreaProps extends ViewProps {
  axis?: "vertical" | "horizontal" | "both";
  onOffset?: (
    offset: { x: number; y: number },
    viewport: number,
  ) => void;
  viewportRef?: NodeRef;
  contentRef?: NodeRef;
  onDragEnd?: () => void;
  onDragBy?: () => void;
  initialOffset?: { x: number; y: number };
  contentMin?: number;
  controller?: ScrollController;
  physics?: ScrollPhysics;
  contentProps?: StyleProps;
}

export interface VirtualListProps<T> extends ElementProps {
  items: readonly T[];
  itemKey: (item: T) => string | number;
  estimatedHeight: number;
  overscan?: number;
  children?: (item: T, index: number) => UIChildren;
  recycle?: {
    typeOf: (item: T) => string;
    render: (item: () => T, index: () => number) => UIChildren;
    poolSize?: number;
  };
  empty?: () => UIChildren;
  error?: string;
  onRetry?: () => void;
  skeleton?: () => UIChildren;
  onEndReached?: () => void;
  /** Distance in points from the end. Defaults to estimated overscan height. */
  endReachedDistance?: number;
  sticky?: (item: T, index: number) => boolean;
  onPin?: (key: string | null) => void;
  restoreKey?: string;
  onRefresh?: () => void;
  refresh?: {
    threshold?: number;
    onRefresh: () => void | Promise<void>;
    indicator?: (state: PullState) => UIChildren;
  };
  controller?: ScrollController;
  physics?: ScrollPhysics;
  viewportProps?: StyleProps;
  contentProps?: StyleProps;
}

export interface RecyclerProps<T> extends ElementProps {
  each: readonly T[];
  keyOf: (item: T) => string | number;
  typeOf: (item: T) => string;
  render: (item: () => T, index: () => number) => UIChildren;
  start?: number;
  poolSize?: number;
  onMounted?: (rows: { key: string; id: number; index: number }[]) => void;
}

export interface RefreshControlProps extends ElementProps {
  controller: ScrollController;
  threshold?: number;
  edge?: "start" | "end";
  onRefresh: () => void | Promise<void>;
  gesture: (handlers: { onDragBy: () => void; onDragEnd: () => void }) => void;
  children?: (state: PullState) => UIChildren;
}

export interface DialogProps extends ElementProps {
  open: boolean;
  onClose: () => void;
  /** Runs after the closing animation has removed the overlay. */
  onClosed?: () => void;
  title?: string;
  initialFocus?: number;
  children?: UIChildren;
  scrimProps?: StyleProps;
  contentProps?: StyleProps;
  titleProps?: StyleProps;
}

export type SheetProps = DialogProps & {
  /** Enables a drag handle that dismisses the sheet on a downward pull. */
  dragToDismiss?: boolean;
};

export interface MenuProps extends ElementProps {
  open: boolean;
  anchor: number;
  onClose: () => void;
  children?: UIChildren;
  contentProps?: StyleProps;
}

export type PopoverProps = MenuProps;

export interface TooltipProps extends ElementProps {
  open: boolean;
  anchor: number;
  text: string;
  textProps?: StyleProps;
}

export interface ToastProps extends ElementProps {
  open: boolean;
  text: string;
  durationMs?: number;
  onClose: () => void;
  textProps?: StyleProps;
}

export interface KeyboardAvoidingProps extends ViewProps {
  behavior?: "padding" | "scroll";
  target?: number;
}

export interface ErrorPageProps extends ViewProps {
  message: string;
  stack?: string;
  retry: () => void;
  onBack?: () => void;
  textProps?: StyleProps;
  actionProps?: StyleProps;
}

export interface OverlayRootProps extends ViewProps {}
