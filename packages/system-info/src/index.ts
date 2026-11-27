import { driver } from "./driver";
export type * from "./types";

export const getAppInfo = (...args: Parameters<ReturnType<typeof driver>["getAppInfo"]>) => driver().getAppInfo(...args);
export const getDeviceInfo = (...args: Parameters<ReturnType<typeof driver>["getDeviceInfo"]>) => driver().getDeviceInfo(...args);
/** On Apple platforms, use the disk capacity fields to support user-visible storage management. */
export const getRuntimeInfo = (...args: Parameters<ReturnType<typeof driver>["getRuntimeInfo"]>) => driver().getRuntimeInfo(...args);
