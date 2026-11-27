import { installMock } from "./driver";
import type { NativePermissionName, PermissionStatus, ServiceStatus } from "./types";

export type PermissionDecision = "grant" | "grantOnce" | "selectPhotos" | "deny" | "doNotAskAgain";
export interface PermissionMockState {
  status?: PermissionStatus;
  serviceStatus?: ServiceStatus;
  shouldShowRequestRationale?: boolean;
}

export function mockPermissions(initial: Partial<Record<NativePermissionName, PermissionMockState>> = {}) {
  const states = new Map(Object.entries(initial).map(([name, state]) => [name, { ...state }]));
  const choices = new Map<NativePermissionName, PermissionDecision[]>();
  const once = new Set<NativePermissionName>();
  const settingsOpened: (NativePermissionName | undefined)[] = [];
  const status = (name: NativePermissionName) => states.get(name)?.status ?? "denied";

  function configure(name: NativePermissionName, state: PermissionMockState) {
    states.set(name, { ...states.get(name), ...state });
    if (state.status !== undefined) once.delete(name);
  }

  function request(name: NativePermissionName): PermissionStatus {
    const current = status(name);
    if (current === "granted" || current === "restricted" || current === "permanentlyDenied") return current;
    const choice = choices.get(name)?.shift();
    if (!choice) return current;
    const outcomes = {
      grant: "granted",
      grantOnce: "granted",
      selectPhotos: "limited",
      deny: "denied",
      doNotAskAgain: "permanentlyDenied",
    } as const satisfies Record<PermissionDecision, PermissionStatus>;
    const next = outcomes[choice];
    configure(name, { status: next, shouldShowRequestRationale: choice === "deny" });
    if (choice === "grantOnce") once.add(name);
    return next;
  }

  const restore = installMock({
    async checkPermissionStatus(name) { return status(name); },
    async checkServiceStatus(name) { return states.get(name)?.serviceStatus ?? "notApplicable"; },
    async shouldShowRequestPermissionRationale(name) { return states.get(name)?.shouldShowRequestRationale ?? false; },
    async requestPermissions(names) { return Object.fromEntries(names.map((name) => [name, request(name)])); },
    async requestNotification() { return request("notification"); },
    async openAppSettings() { settingsOpened.push(undefined); return true; },
    async openSystemSettings(name) { settingsOpened.push(name); return true; },
    async presentLimitedLibraryPicker() { return status("photos") === "limited"; },
  });

  return {
    configure,
    respondWith(name: NativePermissionName, ...decisions: PermissionDecision[]) {
      choices.set(name, [...(choices.get(name) ?? []), ...decisions]);
    },
    expireOneTimeGrants() {
      for (const name of once) configure(name, { status: "denied" });
    },
    get settingsOpened(): readonly (NativePermissionName | undefined)[] { return settingsOpened.slice(); },
    restore,
  };
}
