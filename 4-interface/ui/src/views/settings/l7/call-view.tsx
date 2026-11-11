// What a Layer 7 write shows next to its control: the consent panel while the node waits for an answer (its text verbatim, with Confirm and
// Cancel), the node's problem, and a polite note after it went through. One element for the three, so a section cannot render the control and
// forget the panel: every `useRelayCall()` has exactly one `<CallOutput>` (test/settings-l7.test.ts scans for it).
import { Feedback } from "../row";
import type { RelayCall } from "./actions";

export function CallOutput({ call, problem = null }: { call: RelayCall; /** A problem the control found before it called the node (a bad input); shown in place of the node's. */ problem?: string | null }) {
  return (
    <>
      {call.dialog ? <div className="mt-3">{call.dialog}</div> : null}
      <Feedback text={problem ?? call.state.error} problem />
      <Feedback text={call.state.note} />
    </>
  );
}
