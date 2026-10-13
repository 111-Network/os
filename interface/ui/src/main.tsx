// Entry. U2 (app shell) owns this file; the scaffold only mounts a placeholder.
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./styles/globals.css";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <main className="p-4 text-sm text-muted-foreground">111 node — dashboard scaffold</main>
  </StrictMode>,
);
