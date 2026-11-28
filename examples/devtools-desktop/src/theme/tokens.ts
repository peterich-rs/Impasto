import { createContext, useContext } from "solid-js";

const dark = {
  canvas: "#0D1014", panel: "#14181F", elevated: "#1C222C", hover: "#232A36", selected: "#1E324D",
  border: "#262D3A", strong: "#364154", primary: "#E6ECF5", secondary: "#9AA6B8", muted: "#8492A6",
  accent: "#4C9AFF", ok: "#2EC486", warn: "#F0B429", danger: "#F05D5E", info: "#5AC8FA",
};

const light: typeof dark = {
  canvas: "#F5F7FA", panel: "#FFFFFF", elevated: "#EDF1F7", hover: "#E4E9F2", selected: "#DCEAFE",
  border: "#D8DFEA", strong: "#B8C4D6", primary: "#121822", secondary: "#4C596E", muted: "#596980",
  accent: "#1D6FE8", ok: "#13784D", warn: "#926000", danger: "#C12A31", info: "#066DAD",
};

export function palette(brightness: "dark" | "light") {
  const theme = brightness === "dark" ? dark : light;
  const buckets: string[] = [];
  const put = (first: number, entries: readonly string[]) => {
    for (let index = 0; index < entries.length; index++) buckets[first + index] = entries[index];
  };
  if (brightness === "dark") {
    put(0, ["#F5B041", "#8B94FD", "#2DD4BF", "#64748B", "#3B82F6", "#22C55E", "#38BDF8", theme.danger]);
    put(16, ["#64748B", "#94A3B8", "#22D3EE", "#FB923C", "#A78BFA", "#60A5FA", "#34D399", "#3B82F6", theme.danger]);
    put(32, ["#3B82F6", "#F59E0B", theme.danger, "#C084FC"]);
    put(48, ["#3B82F6", "#22C55E", "#2DD4BF", "#A78BFA", "#F59E0B", "#FB7185", "#64748B"]);
    put(64, ["#3B82F6", "#2DD4BF", "#8B5CF6", "#F59E0B", "#64748B"]);
  } else {
    put(0, ["#BC6407", "#555CE0", "#0D9488", "#7C8EA3", "#2563EB", "#168743", "#0284C7", theme.danger]);
    put(16, ["#64748B", "#94A3B8", "#0891B2", "#C44909", "#7C3AED", "#3B82F6", "#059669", "#1D4ED8", theme.danger]);
    put(32, ["#2563EB", "#B96D06", theme.danger, "#9333EA"]);
    put(48, ["#2563EB", "#168743", "#0D9488", "#7C3AED", "#B96D06", "#C93668", "#64748B"]);
    put(64, ["#2563EB", "#0D9488", "#7439C3", "#B96D06", "#64748B"]);
  }
  put(80, [theme.border, theme.danger, theme.secondary, brightness === "dark" ? "#4C9AFF30" : "#1D6FE829", theme.accent, theme.secondary]);
  return { ...theme, buckets };
}

export const Tokens = createContext(() => palette("dark"));
export const useTokens = () => useContext(Tokens);
export const mono = { fontFamily: "Monospace" } as const;
