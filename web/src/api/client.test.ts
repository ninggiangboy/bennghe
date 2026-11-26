// SPDX-License-Identifier: AGPL-3.0-only
import { afterEach, describe, expect, it, vi } from "vitest";
import { ApiError, api, setCsrfToken, unwrap } from "./client";

const json = (status: number, body: unknown) =>
  new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });

afterEach(() => {
  vi.unstubAllGlobals();
  setCsrfToken(undefined);
});

describe("api client", () => {
  it("sends the CSRF token on writes but not on reads", async () => {
    const seen: Record<string, string | null> = {};
    vi.stubGlobal("fetch", async (req: Request) => {
      seen[req.method] = req.headers.get("x-csrf-token");
      return json(200, []);
    });
    setCsrfToken("abc123");
    await api.GET("/api/v1/projects");
    await api.POST("/api/v1/projects", { body: { name: "A", slug: "a" } });
    expect(seen).toEqual({ GET: null, POST: "abc123" });
  });

  it("turns an error body into an ApiError carrying the stable code", async () => {
    vi.stubGlobal("fetch", async () => json(403, { code: "REAUTH_REQUIRED", message: "confirm your password" }));
    const failure = await (async () => unwrap(await api.GET("/api/v1/projects")))().catch((e: unknown) => e);
    expect(failure).toBeInstanceOf(ApiError);
    expect(failure).toMatchObject({ status: 403, code: "REAUTH_REQUIRED", message: "confirm your password" });
  });
});
