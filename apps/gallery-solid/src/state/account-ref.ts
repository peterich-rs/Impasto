import type { Account } from "./account.ts";

/** App-scope account handle. Set once by the app shell. */
let ref: Account | null = null;

export function setAccount(next: Account): void {
  ref = next;
}

export function accountRef(): Account {
  if (ref === null) {
    throw new Error("account not wired");
  }
  return ref;
}
