/** Route patterns: `:param` and a trailing `*`. */

export function routePath(url: string): string {
  const trimmed = url.trim();
  if (trimmed.length === 0) {
    return "/";
  }
  if (trimmed.includes("://")) {
    try {
      return new URL(trimmed).pathname || "/";
    } catch {
      return "/";
    }
  }
  return trimmed.startsWith("/") ? trimmed : `/${trimmed}`;
}

export function matchPath(pattern: string, path: string): Record<string, string> | null {
  const tokens = pattern.split("/").filter((part) => part.length > 0);
  const parts = routePath(path).split("/").filter((part) => part.length > 0);
  const params: Record<string, string> = {};
  let index = 0;
  for (; index < tokens.length; index += 1) {
    const token = tokens[index] ?? "";
    if (token === "*") {
      if (index !== tokens.length - 1) {
        return null;
      }
      params["*"] = parts.slice(index).join("/");
      return params;
    }
    if (token.startsWith(":")) {
      const value = parts[index];
      if (value === undefined) {
        return null;
      }
      params[token.slice(1)] = decodeURIComponent(value);
      continue;
    }
    if (token !== parts[index]) {
      return null;
    }
  }
  if (index !== parts.length) {
    return null;
  }
  return params;
}
