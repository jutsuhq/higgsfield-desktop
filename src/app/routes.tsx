import {
  createRootRoute,
  createRoute,
  createRouter,
  lazyRouteComponent,
  redirect,
} from "@tanstack/react-router";

import { CreatePage } from "../features/create/CreatePage";
import { QueuePage } from "../features/queue/QueuePage";
import { AppShell } from "./AppShell";
import { RootLayout } from "./RootLayout";

const rootRoute = createRootRoute({ component: RootLayout });

const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  beforeLoad: () => {
    // eslint-disable-next-line @typescript-eslint/only-throw-error -- the router's redirect is thrown by design
    throw redirect({ to: "/create" });
  },
});

const shellRoute = createRoute({ getParentRoute: () => rootRoute, id: "shell", component: AppShell });

const createPageRoute = createRoute({
  getParentRoute: () => shellRoute,
  path: "/create",
  component: CreatePage,
});
const queueRoute = createRoute({ getParentRoute: () => shellRoute, path: "/queue", component: QueuePage });
const libraryRoute = createRoute({
  getParentRoute: () => shellRoute,
  path: "/library",
  component: lazyRouteComponent(() => import("../features/library/LibraryPage"), "LibraryPage"),
});
const libraryResultRoute = createRoute({
  getParentRoute: () => shellRoute,
  path: "/library/result/$id",
  component: lazyRouteComponent(() => import("../features/library/ResultPage"), "ResultPage"),
});
const libraryCompareRoute = createRoute({
  getParentRoute: () => shellRoute,
  path: "/library/compare/$ids",
  component: lazyRouteComponent(() => import("../features/compare/ComparePage"), "ComparePage"),
});
const libraryCompareOriginalRoute = createRoute({
  getParentRoute: () => shellRoute,
  path: "/library/compare-original/$id",
  component: lazyRouteComponent(() => import("../features/compare/ComparePage"), "CompareOriginalPage"),
});
const libraryImageCompareRoute = createRoute({
  getParentRoute: () => shellRoute,
  path: "/library/compare-images/$ids",
  component: lazyRouteComponent(() => import("../features/compare/ImageComparePage"), "ImageComparePage"),
});
const libraryImageOriginalRoute = createRoute({
  getParentRoute: () => shellRoute,
  path: "/library/compare-image-original/$id",
  component: lazyRouteComponent(() => import("../features/compare/ImageComparePage"), "ImageOriginalPage"),
});
const libraryImageVariantsRoute = createRoute({
  getParentRoute: () => shellRoute,
  path: "/library/compare-variants/$group",
  component: lazyRouteComponent(() => import("../features/compare/ImageComparePage"), "ImageVariantsPage"),
});
const libraryBatchRoute = createRoute({
  getParentRoute: () => shellRoute,
  path: "/library/batch/$id",
  component: lazyRouteComponent(() => import("../features/compare/BatchPage"), "BatchPage"),
});
const settingsRoute = createRoute({
  getParentRoute: () => shellRoute,
  path: "/settings",
  component: lazyRouteComponent(() => import("../features/settings/SettingsPage"), "SettingsPage"),
});

const onboardingRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/onboarding",
  component: lazyRouteComponent(() => import("../features/onboarding/OnboardingPage"), "OnboardingPage"),
});

/** The tray panel's own window. */
const trayRoute = createRoute({ getParentRoute: () => rootRoute, path: "/tray", component: lazyRouteComponent(() => import("../features/tray/TrayPanel"), "TrayPanel") });

/** The About window (`AboutView`). */
const aboutRoute = createRoute({ getParentRoute: () => rootRoute, path: "/about", component: lazyRouteComponent(() => import("../features/about/AboutPage"), "AboutPage") });

/** Every control in every state, for visual review. Development builds only: production drops the page. */
const devRoutes = import.meta.env.DEV
  ? [
      createRoute({
        getParentRoute: () => rootRoute,
        path: "/design",
        component: lazyRouteComponent(() => import("./DesignPage"), "DesignPage"),
      }),
    ]
  : [];

export const routeTree = rootRoute.addChildren([
  indexRoute,
  shellRoute.addChildren([createPageRoute, queueRoute, libraryRoute, settingsRoute, libraryResultRoute, libraryCompareRoute, libraryCompareOriginalRoute, libraryImageCompareRoute, libraryImageOriginalRoute, libraryImageVariantsRoute, libraryBatchRoute]),
  onboardingRoute,
  trayRoute,
  aboutRoute,
  ...devRoutes,
]);

export const router = createRouter({ routeTree });

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
