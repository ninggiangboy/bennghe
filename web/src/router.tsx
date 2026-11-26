// SPDX-License-Identifier: AGPL-3.0-only
import { useQueryClient, type QueryClient } from "@tanstack/react-query";
import { Link, Outlet, createRootRouteWithContext, createRoute, createRouter, redirect, useNavigate } from "@tanstack/react-router";
import { api, setCsrfToken, unwrap } from "@/api/client";
import { projectsQuery, sessionQuery } from "@/api/session";
import { ReauthProvider } from "@/components/reauth";
import { Button } from "@/components/ui/button";
import { LoginPage } from "@/pages/login";
import { ProjectsPage } from "@/pages/projects";
import { SecurityPage } from "@/pages/security";
import { SetupPage } from "@/pages/setup";

interface RouterContext {
  queryClient: QueryClient;
}

const rootRoute = createRootRouteWithContext<RouterContext>()({ component: Outlet });

const loginRoute = createRoute({ getParentRoute: () => rootRoute, path: "/login", component: LoginPage });
const setupRoute = createRoute({ getParentRoute: () => rootRoute, path: "/setup", component: SetupPage });

// Everything below needs a signed-in user. Signed-out visitors go to setup on a fresh install
// and to the sign-in page otherwise; users who still owe a second factor go to enrolment.
const appRoute = createRoute({
  getParentRoute: () => rootRoute,
  id: "app",
  beforeLoad: async ({ context, location }) => {
    const session = await context.queryClient.ensureQueryData(sessionQuery);
    if (!session) {
      const status = unwrap(await api.GET("/api/v1/setup/status"));
      throw redirect({ to: status.needs_setup ? "/setup" : "/login" });
    }
    if (session.must_enrol_2fa && location.pathname !== "/security") throw redirect({ to: "/security" });
    return { session };
  },
  loader: ({ context }) => context.queryClient.ensureQueryData(projectsQuery),
  component: AppShell,
});

const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  beforeLoad: () => {
    throw redirect({ to: "/projects" });
  },
});
const projectsRoute = createRoute({ getParentRoute: () => appRoute, path: "/projects", component: ProjectsPage });
const securityRoute = createRoute({ getParentRoute: () => appRoute, path: "/security", component: SecurityPage });

const routeTree = rootRoute.addChildren([indexRoute, loginRoute, setupRoute, appRoute.addChildren([projectsRoute, securityRoute])]);

export const router = createRouter({ routeTree, context: { queryClient: undefined! }, defaultPreload: "intent" });

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}

function AppShell() {
  const { session } = appRoute.useRouteContext();
  const queryClient = useQueryClient();
  const navigate = useNavigate();

  const signOut = async () => {
    await api.POST("/api/v1/auth/logout");
    setCsrfToken(undefined);
    queryClient.clear();
    await navigate({ to: "/login" });
  };

  return (
    <ReauthProvider totpEnabled={session.totp_enabled}>
      <header className="border-b border-border bg-card">
        <div className="mx-auto flex max-w-4xl items-center justify-between gap-4 px-4 py-3">
          <nav className="flex items-center gap-4 text-sm">
            <strong>Bennghe</strong>
            {session.must_enrol_2fa ? null : (
              <Link to="/projects" className="text-muted-foreground hover:text-foreground [&.active]:text-foreground">Projects</Link>
            )}
            <Link to="/security" className="text-muted-foreground hover:text-foreground [&.active]:text-foreground">Security</Link>
          </nav>
          <div className="flex items-center gap-3 text-sm">
            <span className="text-muted-foreground">{session.user.email}</span>
            <Button variant="outline" size="sm" onClick={() => void signOut()}>Sign out</Button>
          </div>
        </div>
      </header>
      <main className="mx-auto max-w-4xl px-4 py-8">
        <Outlet />
      </main>
    </ReauthProvider>
  );
}
