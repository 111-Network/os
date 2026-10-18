// Entry. Guards old browsers, starts the data layer, then renders the app. While the adapter loads
// nothing is rendered: the notice in index.html is enough.
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./styles/globals.css";
import App from "./app/App";
import { Empty } from "./components/empty";
import { startApi } from "./lib/api";

declare global {
  interface Window {
    /** Set by the feature detect in index.html; the body has already been replaced by a notice. */
    __111_unsupported?: boolean;
  }
}

async function boot(): Promise<void> {
  const container = document.getElementById("root");
  if (window.__111_unsupported || !container) return;
  const root = createRoot(container);
  try {
    await startApi();
  } catch (err) {
    root.render(
      <StrictMode>
        <div className="p-4">
          <Empty kicker="startup" title="The dashboard could not start">
            {err instanceof Error ? err.message : "Unknown error while loading the data layer."}
          </Empty>
        </div>
      </StrictMode>,
    );
    return;
  }
  root.render(
    <StrictMode>
      <App />
    </StrictMode>,
  );
}

void boot();
