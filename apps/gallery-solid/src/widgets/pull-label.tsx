import { Text, type NodeHandle } from "@impasto/solid-renderer";

const STOPS: [number, number, number, number][] = [
  [0xe2, 0x4b, 0x4b, 255],
  [0xe0, 0x7a, 0x2f, 255],
  [0xc4, 0xa0, 0x35, 255],
  [0x3e, 0x8f, 0x5a, 255],
  [0x2f, 0x8f, 0x8a, 255],
  [0x3c, 0x6f, 0xbf, 255],
  [0x7a, 0x4e, 0x9a, 255],
];

/**
 * Brand word in the pull gap. The control clips this to the open gap, so the
 * runs stay fixed: recoloring on every sample rebuilt the paragraph mid-drag.
 */
export function PullLabel(_props: { progress: () => number }): NodeHandle {
  return Text({
    fontSize: 20,
    weight: 700,
    runs: [
      {
        text: "Impasto",
        color: STOPS[0],
        gradient: {
          kind: "linear" as const,
          from: [0, 0] as [number, number],
          to: [1, 0] as [number, number],
          stops: STOPS.map((color, index) => ({
            offset: index / (STOPS.length - 1),
            color,
          })),
        },
      },
    ],
  });
}
