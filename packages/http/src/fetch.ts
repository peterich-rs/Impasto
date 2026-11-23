import * as native from "./native.js";
import {
  createFetch,
  Request,
  Response,
  type RequestInfo,
  type RequestInit,
  type ResponseInit,
} from "./fetch-impl.ts";
import { FormData, type FormDataEntryValue } from "./form-data.ts";
import { Headers, type HeadersInit } from "./headers.ts";

export { FormData, Headers, Request, Response };
export type {
  FormDataEntryValue,
  HeadersInit,
  RequestInfo,
  RequestInit,
  ResponseInit,
};

export const fetch = createFetch(native);
