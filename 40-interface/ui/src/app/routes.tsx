// Route table (spec section 5). Every view is a lazily loaded module at a fixed path under
// src/views/<name>/index.tsx; later tasks replace the bodies. `/devices/*`, `/settings/*` and
// `/kb/*` all land in one view, which reads its own params (useParams / useRoute).
import { lazy, Suspense, type ComponentType, type ReactNode } from "react";
import { Route, Switch, useLocation } from "wouter";
import { Badge } from "../components/badge";
import { ButtonLink } from "../components/button";
import { Empty } from "../components/empty";
import { useMode } from "../lib/mode";
import { minModeFor } from "../lib/nav";
import { hrefTo } from "./location";

const Agent = lazy(() => import("../views/agent"));
const Overview = lazy(() => import("../views/overview"));
const Devices = lazy(() => import("../views/devices"));
const Nearby = lazy(() => import("../views/nearby"));
const Messages = lazy(() => import("../views/messages"));
// Foreign networks and fan-out (Layer 6): their own lazy chunk, under D21's 64 KB cap.
const MessagesNet = lazy(() => import("../views/messages/net"));
const Contacts = lazy(() => import("../views/contacts"));
const Services = lazy(() => import("../views/services"));
const Settings = lazy(() => import("../views/settings"));
const Licenses = lazy(() => import("../views/licenses"));
const Logs = lazy(() => import("../views/logs"));
const Kb = lazy(() => import("../views/kb"));
// Dev gallery: the condition is a build-time constant, so production builds drop the import and the chunk.
const Gallery = import.meta.env.DEV ? lazy(() => import("../views/_gallery")) : null;

export interface RouteDef {
  path: string;
  View: ComponentType;
}

export const ROUTES: RouteDef[] = [
  { path: "/", View: Agent },
  { path: "/overview", View: Overview },
  { path: "/devices", View: Devices },
  { path: "/devices/add", View: Devices },
  { path: "/devices/:id", View: Devices },
  { path: "/nearby", View: Nearby },
  { path: "/messages", View: Messages },
  { path: "/messages/contacts", View: Contacts }, // before /messages/:peer, which would call it "no such conversation"
  { path: "/messages/send", View: MessagesNet }, // Layer 6 fan-out: before /messages/:peer for the same reason
  { path: "/messages/net/:network", View: MessagesNet }, // Layer 6 foreign networks: before /messages/:peer (a network id is not a node id)
  { path: "/messages/net/:network/:conv", View: MessagesNet },
  { path: "/messages/:peer", View: Messages },
  { path: "/services/:id", View: Services },
  { path: "/logs", View: Logs },
  { path: "/settings", View: Settings },
  { path: "/settings/licenses", View: Licenses }, // before /settings/:layer, which would call it "no such layer"
  { path: "/settings/:layer", View: Settings },
  { path: "/kb", View: Kb },
  { path: "/kb/*", View: Kb },
  ...(Gallery ? [{ path: "/_gallery", View: Gallery }] : []),
];

/** An advanced-only view opened by deep link in simple mode: shown, never redirected, and marked. */
function Frame({ children }: { children: ReactNode }) {
  const [path] = useLocation();
  const { mode } = useMode();
  const marked = mode === "simple" && minModeFor(path) === "advanced";
  return (
    <>
      {marked ? (
        <div className="mb-4">
          <Badge>advanced view</Badge>
        </div>
      ) : null}
      {children}
    </>
  );
}

export function NotFound() {
  return (
    <Empty
      kicker="404"
      title="No such view"
      action={
        <ButtonLink href={hrefTo("/")} size="sm">
          Go to Agent
        </ButtonLink>
      }
    >
      This address does not match any page of the dashboard.
    </Empty>
  );
}

function Loading() {
  return (
    <p role="status" className="sr-only">
      Loading view
    </p>
  );
}

export function AppRoutes() {
  return (
    <Suspense fallback={<Loading />}>
      <Switch>
        {ROUTES.map(({ path, View }) => (
          <Route key={path} path={path}>
            <Frame>
              <View />
            </Frame>
          </Route>
        ))}
        <Route>
          <NotFound />
        </Route>
      </Switch>
    </Suspense>
  );
}
