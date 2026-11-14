import { Outlet, useNavigate, useRouterState } from "@tanstack/react-router";
import { useEffect } from "react";

import { SignInSheet } from "../features/onboarding/SignInSheet";
import { ConnectionAlert } from "./ConnectionAlert";
import { useConnection, useLiveEvents } from "./liveData";
import { pages } from "./screens";

/**
 * The window (MainWindow.swift): follows Rust's events, sends a window without a connection to onboarding and a
 * connected one on to Create, and keeps the sign-in sheet and the connection's alert over every page. Settings
 * stays reachable without a connection, from onboarding.
 */
export function RootLayout() {
  useLiveEvents();
  const connection = useConnection();
  // Unknown while the vault is still read at launch: neither onboarding nor Create yet.
  const kind = connection === undefined || connection.isRestoring ? undefined : connection.kind;
  const pathname = useRouterState({ select: (state) => state.location.pathname });
  const navigate = useNavigate();

  useEffect(() => {
    if (kind === undefined) return;
    const isPage = pages.some((page) => pathname === page.path || pathname.startsWith(`${page.path}/`));
    if (kind === null && isPage) void navigate({ to: "/onboarding", replace: true });
    if (kind !== null && pathname === "/onboarding") void navigate({ to: "/create", replace: true });
  }, [kind, pathname, navigate]);

  // The tray panel and About are windows of their own: the sheet and the alert belong to the main window.
  if (pathname === "/tray" || pathname === "/about") return <Outlet />;
  return (
    <>
      <Outlet />
      <SignInSheet />
      <ConnectionAlert />
    </>
  );
}
