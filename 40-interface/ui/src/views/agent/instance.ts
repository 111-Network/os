// The one chat session for this page load, wired to the real adapter, the shell's announcer and the
// sign-in redirect. It lives at module level so a reply keeps streaming while another page is open.
import { redirectIfUnauthenticated } from "../../app/AuthGate";
import { announce } from "../../app/LiveRegion";
import { getAdapter } from "../../lib/api";
import { ChatSession } from "./session";

export const chatSession = new ChatSession({
  getAdapter,
  announce,
  onUnauthenticated: redirectIfUnauthenticated,
});
