import { Canvas } from "@impasto/solid-ui";
import { Path2D } from "@impasto/solid-renderer";
import { useTokens } from "../theme/tokens";

const paths = {
  overview: "M2 12h5l3-6 4 12 3-6h5",
  frame: "M3 4h18v16H3z M3 9h18 M3 15h18 M8 4v16 M16 4v16",
  view: "M4 5h16v5H4z M7 10v7h4 M11 15h9v5h-9z",
  js: "M8 5h8l3 3v8l-3 3H8l-3-3V8z M9 9h6v6H9z M9 2v3 M15 2v3 M9 19v3 M15 19v3 M2 9h3 M2 15h3 M19 9h3 M19 15h3",
  network: "M3 7h17l-4-4 M20 7l-4 4 M21 17H4l4-4 M4 17l4 4",
  resources: "M4 4h16v16H4z M4 11h16 M10 11v9 M14 4v7",
  terminal: "M3 5h18v14H3z M7 9l3 3-3 3 M13 15h4",
  target: "M5 5h14v11H5z M9 20h6 M12 16v4",
  panel: "M3 5h18v14H3z M15 5v14",
  chevron: "M9 5l7 7-7 7",
  left: "M15 5l-7 7 7 7",
  close: "M6 6l12 12 M18 6 6 18",
  record: "M12 5a7 7 0 1 0 0 14 7 7 0 0 0 0-14z",
  stop: "M6 6h12v12H6z",
  pause: "M9 5v14 M15 5v14",
  play: "M8 5l11 7-11 7z",
  search: "M15 15l5 5 M16 10a6 6 0 1 0-12 0 6 6 0 0 0 12 0z",
  open: "M3 7h7l2 3h9l-3 9H3z",
  export: "M12 3v12 M8 11l4 4 4-4 M4 16v5h16v-5",
  pick: "M12 2v5 M12 17v5 M2 12h5 M17 12h5 M12 7a5 5 0 1 0 0 10 5 5 0 0 0 0-10z",
  warning: "M12 4l9 16H3z M12 10v4 M12 17v.3",
  refresh: "M19 9a7 7 0 1 0 0 7 M19 4v5h-5",
};

export type Glyph = keyof typeof paths;
const shapes = new Map<Glyph, Path2D>();
export function Icon(props: { name: Glyph; size?: number; color?: string }) {
  const tokens = useTokens();
  const size = () => props.size ?? 16;
  return <Canvas style={{ width: size(), height: size(), flexShrink: 0 }} draw={(ctx, dimensions) => {
    let path = shapes.get(props.name);
    if (!path) { path = new Path2D(paths[props.name]); shapes.set(props.name, path); }
    ctx.scale(dimensions.width / 24, dimensions.height / 24);
    ctx.strokeStyle = props.color ?? tokens().secondary;
    ctx.lineWidth = 1.65;
    ctx.lineCap = "round";
    ctx.lineJoin = "round";
    ctx.stroke(path);
  }} />;
}
