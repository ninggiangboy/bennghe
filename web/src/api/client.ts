// SPDX-License-Identifier: AGPL-3.0-only
import createClient from "openapi-fetch";
import type { paths } from "./schema";

/** Error body every API failure carries. `code` is stable; `message` is for people. */
export class ApiError extends Error {
  constructor(
    public readonly status: number,
    public readonly code: string,
    message: string,
  ) {
    super(message);
  }
}

// The CSRF token comes with the session and must accompany every request that changes data.
let csrfToken: string | undefined;
export const setCsrfToken = (token: string | undefined) => {
  csrfToken = token;
};

const SAFE_METHODS = new Set(["GET", "HEAD", "OPTIONS"]);

// `fetch` is looked up per request so it can be replaced in tests; the base URL is the page origin.
export const api = createClient<paths>({
  baseUrl: globalThis.location?.origin ?? "",
  credentials: "same-origin",
  fetch: (request) => globalThis.fetch(request),
});

api.use({
  onRequest({ request }) {
    if (csrfToken && !SAFE_METHODS.has(request.method)) request.headers.set("X-CSRF-Token", csrfToken);
    return request;
  },
});

type Result<T> = { data?: T; error?: unknown; response: Response };

/** Returns the data or throws an `ApiError` built from the error body. */
export function unwrap<T>({ data, error, response }: Result<T>): T {
  if (response.ok) return data as T;
  const body = (error ?? {}) as { code?: string; message?: string };
  throw new ApiError(response.status, body.code ?? "UNKNOWN", body.message ?? response.statusText);
}

export const errorMessage = (e: unknown) => (e instanceof Error ? e.message : "Something went wrong");
