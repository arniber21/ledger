import { createRootRoute, createRoute, createRouter, RouterProvider } from "@tanstack/react-router";
import { isSavedView } from "./stream/reduce";
import { Dashboard, parseOutcomeSearch } from "./dashboard/dashboard";
import { InboxPage, VerdictsPage } from "./pages/places";
import { SpecsPage } from "./specs/specs";
import { TracesPage } from "./traces/traces";
import { Shell } from "./shell/shell";

const rootRoute = createRootRoute({
  component: Shell,
});

const dashboardRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  validateSearch: (search: Record<string, unknown>) => ({
    view: isSavedView(search.view) ? search.view : undefined,
    outcome: parseOutcomeSearch(search.outcome),
  }),
  component: function DashboardRoute() {
    const search = dashboardRoute.useSearch();
    return <Dashboard view={search.view} outcome={search.outcome} />;
  },
});

const inboxRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/inbox",
  component: InboxPage,
});

const tracesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/traces",
  validateSearch: (search: Record<string, unknown>) => ({
    trace: typeof search.trace === "string" ? search.trace : undefined,
  }),
  component: function TracesRoute() {
    const search = tracesRoute.useSearch();
    return <TracesPage traceId={search.trace} />;
  },
});

const specsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/specs",
  validateSearch: (search: Record<string, unknown>) => ({
    spec: typeof search.spec === "string" ? search.spec : undefined,
  }),
  component: function SpecsRoute() {
    const search = specsRoute.useSearch();
    return <SpecsPage spec={search.spec} />;
  },
});

const verdictsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/verdicts",
  validateSearch: (search: Record<string, unknown>) => ({
    id: typeof search.id === "string" ? search.id : undefined,
  }),
  component: function VerdictsRoute() {
    const search = verdictsRoute.useSearch();
    return <VerdictsPage id={search.id} />;
  },
});

const routeTree = rootRoute.addChildren([dashboardRoute, inboxRoute, tracesRoute, specsRoute, verdictsRoute]);

export const router = createRouter({ routeTree });

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}

export function AppRouter() {
  return <RouterProvider router={router} />;
}
