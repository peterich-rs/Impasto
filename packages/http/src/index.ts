import * as native from "@impasto/http/native";
import {
  createClient,
  createLatest,
  HttpClient,
  HttpError,
  type HttpBodyInput,
  type HttpClientDefaults,
  type HttpErrorCode,
  type HttpRequestOptions,
  type HttpResponseData,
  type HttpTransport,
  type RedirectMode,
} from "./client.ts";
import { FormData, type FormDataEntryValue } from "./form-data.ts";
import { Headers, type HeadersInit } from "./headers.ts";

export {
  createClient,
  createLatest,
  FormData,
  Headers,
  HttpClient,
  HttpError,
};
export type {
  FormDataEntryValue,
  HeadersInit,
  HttpBodyInput,
  HttpClientDefaults,
  HttpErrorCode,
  HttpRequestOptions,
  HttpResponseData,
  HttpTransport,
  RedirectMode,
};

export const http: HttpClient = createClient(native);
export default http;
