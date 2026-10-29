let text = "";

export function getString(): string {
  return text;
}

export function setString(value: string): void {
  text = value;
}

export function addEventListener(
  _type: string,
  _listener: (e: unknown) => void,
): void {}

export function removeEventListener(
  _type: string,
  _listener: (e: unknown) => void,
): void {}
