import { Canvas, Path2D, Pressable, Role } from "@impasto/solid-ui";
import type { NodeHandle, ViewStyle } from "@impasto/solid-renderer";
import { loc } from "../../copy.ts";
import { colors } from "./theme.ts";

export type IconName =
  | "back"
  | "star"
  | "starFilled"
  | "pause"
  | "play"
  | "close"
  | "more"
  | "chevronDown"
  | "chevronUp"
  | "search"
  | "plus"
  | "minus"
  | "wallet"
  | "gear";

export type IconColor = string | (() => string);

/**
 * Vector glyph on a 24×24 grid, scaled to `size`. Drawn through the same
 * Canvas path as the sparklines so there are no bitmap assets to ship.
 */
export function Icon(props: { name: IconName; size?: number; color?: IconColor }) {
  const side = () => props.size ?? 20;
  const tint = () => (typeof props.color === "function" ? props.color() : (props.color ?? colors.ink));
  return <Canvas style={{ width: side(), height: side(), flexShrink: 0 }}
    draw={(ctx, { width }) => {
      const color = tint();
      if (color.length === 0) return;
      const scale = width / 24;
      ctx.save();
      ctx.scale(scale, scale);
      ctx.strokeStyle = color;
      ctx.fillStyle = color;
      ctx.lineWidth = 2;
      ctx.lineCap = "round";
      ctx.lineJoin = "round";
      const path = new Path2D();
      switch (props.name) {
        case "back":
          path.moveTo(14.5, 5.5); path.lineTo(8, 12); path.lineTo(14.5, 18.5);
          ctx.stroke(path);
          break;
        case "chevronDown":
          path.moveTo(6, 9.5); path.lineTo(12, 15.5); path.lineTo(18, 9.5);
          ctx.stroke(path);
          break;
        case "chevronUp":
          path.moveTo(6, 14.5); path.lineTo(12, 8.5); path.lineTo(18, 14.5);
          ctx.stroke(path);
          break;
        case "close":
          path.moveTo(7, 7); path.lineTo(17, 17);
          path.moveTo(17, 7); path.lineTo(7, 17);
          ctx.stroke(path);
          break;
        case "plus":
          path.moveTo(12, 6); path.lineTo(12, 18);
          path.moveTo(6, 12); path.lineTo(18, 12);
          ctx.stroke(path);
          break;
        case "minus":
          path.moveTo(6, 12); path.lineTo(18, 12);
          ctx.stroke(path);
          break;
        case "search":
          path.arc(10.5, 10.5, 5.5, 0, Math.PI * 2);
          ctx.stroke(path);
          ctx.lineWidth = 2.4;
          path.moveTo(15, 15); path.lineTo(19.5, 19.5);
          ctx.stroke(path);
          break;
        case "more":
          for (const x of [6, 12, 18]) { path.moveTo(x + 1.8, 12); path.arc(x, 12, 1.8, 0, Math.PI * 2); }
          ctx.fill(path);
          break;
        case "pause":
          path.roundRect(7, 5.5, 3.5, 13, 1.5);
          path.roundRect(13.5, 5.5, 3.5, 13, 1.5);
          ctx.fill(path);
          break;
        case "play":
          path.moveTo(8, 5.5); path.lineTo(18.5, 12); path.lineTo(8, 18.5); path.closePath();
          ctx.fill(path);
          break;
        case "star":
        case "starFilled": {
          for (let point = 0; point < 10; point++) {
            const radius = point % 2 === 0 ? 9 : 4.2;
            const angle = -Math.PI / 2 + (point * Math.PI) / 5;
            const x = 12 + radius * Math.cos(angle);
            const y = 12.4 + radius * Math.sin(angle);
            if (point === 0) path.moveTo(x, y); else path.lineTo(x, y);
          }
          path.closePath();
          if (props.name === "starFilled") ctx.fill(path); else ctx.stroke(path);
          break;
        }
        case "wallet": {
          path.roundRect(4, 6, 16, 13, 3);
          ctx.stroke(path);
          path.moveTo(4, 9.5); path.lineTo(20, 9.5);
          ctx.stroke(path);
          path.moveTo(15.5, 14.5); path.arc(15.5, 14.5, 1.4, 0, Math.PI * 2);
          ctx.fill(path);
          break;
        }
        case "gear": {
          path.arc(12, 12, 3.2, 0, Math.PI * 2);
          ctx.stroke(path);
          for (let tooth = 0; tooth < 8; tooth++) {
            const angle = (tooth * Math.PI) / 4;
            path.moveTo(12 + 7 * Math.cos(angle), 12 + 7 * Math.sin(angle));
            path.lineTo(12 + 9.5 * Math.cos(angle), 12 + 9.5 * Math.sin(angle));
          }
          ctx.stroke(path);
          break;
        }
      }
      ctx.restore();
    }} />;
}

/**
 * Icon-only button. The glyph is `size` tall and defaults to a 44×44 hit area.
 */
export function Glyph(props: {
  name: IconName; title: string; onClick: () => void;
  size?: number; color?: IconColor; primary?: boolean; style?: ViewStyle;
  /** 32×32 target for inline actions inside dense rows. Default is the 44 accessible minimum. */
  compact?: boolean;
}): NodeHandle {
  const buttonStyle = (): ViewStyle => {
    const target = props.compact ? 32 : 44;
    return { width: target, height: target, minWidth: target, minHeight: target, padding: 0, flexShrink: 0, alignItems: "center", justifyContent: "center",
      borderRadius: target / 2, backgroundColor: props.primary ? colors.lime : colors.raised, ...props.style };
  };
  return <Pressable role={Role.Button} accessibilityLabel={props.title} onClick={props.onClick} style={buttonStyle()}>
    <Icon name={props.name} size={props.size} color={props.color ?? (props.primary ? colors.bg : undefined)} />
  </Pressable>;
}

/** Back control for pushed pages. */
export function BackButton(props: { onClick: () => void }): NodeHandle {
  return <Glyph title={loc("返回", "Back")} name="back" onClick={props.onClick} />;
}
