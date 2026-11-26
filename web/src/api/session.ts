// SPDX-License-Identifier: AGPL-3.0-only
import { queryOptions } from "@tanstack/react-query";
import { api, setCsrfToken, unwrap } from "./client";
import type { components } from "./schema";

export type SessionInfo = components["schemas"]["SessionInfo"];

/** The signed-in user, or `null` when nobody is signed in. */
export const sessionQuery = queryOptions({
  queryKey: ["session"],
  retry: false,
  staleTime: 30_000,
  queryFn: async (): Promise<SessionInfo | null> => {
    const result = await api.GET("/api/v1/auth/session");
    if (result.response.status === 401) {
      setCsrfToken(undefined);
      return null;
    }
    const session = unwrap(result);
    setCsrfToken(session.csrf_token);
    return session;
  },
});

export const projectsQuery = queryOptions({
  queryKey: ["projects"],
  queryFn: async () => unwrap(await api.GET("/api/v1/projects")),
});
