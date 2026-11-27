import { driver } from "./driver";
import type { OpenUrlOptions, UrlListener } from "./types";
export type * from "./types";

export const getInitialURL = () => driver().getInitialURL();
export const canOpenURL = (url: string) => driver().canOpenURL(url);
export const openURL = (url: string, options?: OpenUrlOptions) => driver().openURL(url, options);
export const watchUrls = (signal?: AbortSignal) => driver().watchUrls(signal);
export const addEventListener = (type: "url", listener: UrlListener) => driver().addEventListener(type, listener);
export const removeEventListener = (type: "url", listener: UrlListener) => driver().removeEventListener(type, listener);

export function mailtoUrl(to: string, options?: { subject?: string; body?: string }): string {
  if (!to.trim() || /[\r\n]/.test(to)) throw new TypeError("mailtoUrl requires an address without line breaks");
  const target = encodeURIComponent(to.trim()).replace(/%40/g, "@");
  const query = [];
  if (options?.subject !== undefined) query.push(`subject=${encodeURIComponent(options.subject)}`);
  if (options?.body !== undefined) query.push(`body=${encodeURIComponent(options.body)}`);
  return query.length ? `mailto:${target}?${query.join("&")}` : `mailto:${target}`;
}

export function telUrl(phoneNumber: string): string {
  const number = phoneNumber.replace(/[\s().-]/g, "");
  if (!/^\+?[0-9*#]+$/.test(number)) throw new TypeError("telUrl requires a telephone number");
  return `tel:${number.replace(/#/g, "%23")}`;
}
export const Linking = Object.freeze({ getInitialURL, canOpenURL, openURL, watchUrls, addEventListener, removeEventListener, mailtoUrl, telUrl });
