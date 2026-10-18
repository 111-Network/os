// A view inside a wouter Router that renders at a fixed path (server rendering has no location): `routed("/services/fabric",
// "/services/:id", ServicesView)` is the page as `#/services/fabric` shows it.
import { createElement as h, type ComponentType, type ReactElement } from "react";
import { Route, Router } from "wouter";

export function routed(ssrPath: string, pattern: string, component: ComponentType): ReactElement {
  return h(Router, { ssrPath, children: h(Route, { path: pattern, component }) });
}
