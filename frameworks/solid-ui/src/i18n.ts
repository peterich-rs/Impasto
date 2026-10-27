import { currentLocale } from "./theme.ts";

export type Messages = {
  switchOn: string;
  switchOff: string;
  retry: string;
  back: string;
  errorTitle: string;
};

const ZH: Messages = {
  switchOn: "开",
  switchOff: "关",
  retry: "重试",
  back: "返回",
  errorTitle: "出错了",
};
const EN: Messages = {
  switchOn: "On",
  switchOff: "Off",
  retry: "Retry",
  back: "Back",
  errorTitle: "Something went wrong",
};

export function messagesFor(locale: string): Messages {
  return locale.startsWith("zh") ? ZH : EN;
}

/** Looks up `key` in the locale table and replaces `{name}` placeholders. */
export function t<K extends keyof Messages>(
  key: K,
  params?: Record<string, string | number>,
  locale: string = currentLocale(),
): string {
  const template = messagesFor(locale)[key];
  if (!params) {
    return template;
  }
  return template.replace(/\{(\w+)\}/g, (_match, name: string) => {
    const value = params[name];
    return value === undefined ? "" : String(value);
  });
}
