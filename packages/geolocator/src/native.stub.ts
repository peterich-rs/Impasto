import type { PermissionStatus } from "@impasto/permissions";
import type { Coordinates, CurrentPositionOptions, LocationServiceStatus, Position, WatchPositionOptions } from "./types";
function unavailable(): never { throw new Error("[geolocator:host-unavailable] Location requires a native host"); }
export const isGeolocatorHostAvailable = () => false;
export const nextRequestId = (): number => unavailable();
export async function getCurrentPosition(_id: number, _options?: CurrentPositionOptions): Promise<Position> { return unavailable(); }
export async function cancel(_id: number) {}
export async function getLastKnownPosition(): Promise<Position | null> { return unavailable(); }
export function watchPosition(_options?: WatchPositionOptions, _signal?: AbortSignal): ReadableStream<Position> { return unavailable(); }
export function watchServiceStatus(_signal?: AbortSignal): ReadableStream<LocationServiceStatus> { return unavailable(); }
export async function isLocationServiceEnabled(): Promise<boolean> { return unavailable(); }
export async function checkPermission(): Promise<PermissionStatus> { return "denied"; }
export async function requestPermission(_scope?: "whenInUse" | "always"): Promise<PermissionStatus> { return "denied"; }
export const distanceBetween = (_a: Coordinates, _b: Coordinates): number => unavailable();
export const bearingBetween = (_a: Coordinates, _b: Coordinates): number => unavailable();
export const destinationPoint = (_origin: Coordinates, _distance: number, _bearing: number): Coordinates => unavailable();
