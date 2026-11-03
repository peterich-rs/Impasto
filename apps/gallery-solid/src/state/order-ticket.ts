/** Side/price for the next `/order/:symbol` push. The router has no query string. */
let ticket: Record<string, string> = {};

export function setOrderTicket(next: Record<string, string>): void {
  ticket = { ...next };
}

export function takeOrderTicket(): Record<string, string> {
  const next = ticket;
  ticket = {};
  return next;
}
