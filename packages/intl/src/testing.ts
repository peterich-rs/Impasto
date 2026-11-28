import { setIntlBackend } from "@impasto/i18n";
import { isIntlHostAvailable } from "@impasto/intl/native";
import { enableI18nIntl } from "./index";

export function useTestIntl(): () => void {
  if (isIntlHostAvailable()) throw new Error("[intl:host-unavailable] Test configuration requires a headless host");
  enableI18nIntl();
  return () => setIntlBackend(undefined);
}
