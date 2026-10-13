// Messages (spec 6.5): a structural stub. No fake inbox; it says what is coming and offers a prompt.
import { useAppLocation } from "../../app/location";
import { Button } from "../../components/button";
import { Empty } from "../../components/empty";
import { SectionHeader } from "../../components/section-header";
import { SpecTable } from "../../components/spec-table";
import { agentHrefFor, stagePrompt } from "../../lib/prompts";

export const MESSAGING_PROMPT = "Explain how messaging will work on this node and what I can do today.";

/** The four honest delivery states. Reliable-first: hours between them is normal on a mesh. */
const DELIVERY_STATES = [
  { label: "queued", value: "Saved on this node, waiting for a path to the other person." },
  { label: "sent", value: "Handed to the mesh by this node." },
  { label: "relayed", value: "A node in between accepted it and is carrying it onward." },
  { label: "delivered", value: "The other person's node confirmed it arrived." },
];

export default function MessagesView() {
  const [, navigate] = useAppLocation();
  const ask = () => navigate(agentHrefFor(stagePrompt(MESSAGING_PROMPT, "Messages", "check")));

  return (
    <div className="space-y-6">
      <SectionHeader level={1} kicker="messages" title="Messages" />
      <Empty
        kicker="coming"
        title="Messages arrive with Layer 5"
        action={
          <Button variant="primary" onClick={ask}>
            Ask the agent about messaging
          </Button>
        }
      >
        Human messaging is the main thing this network is for, and it is not built yet. When it lands you will find conversations and contacts here. Nothing on this page is a real
        message today.
      </Empty>
      <section aria-labelledby="delivery-states" className="space-y-2">
        <h2 id="delivery-states" className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground">
          how delivery will be shown
        </h2>
        <SpecTable rows={DELIVERY_STATES} />
        <p className="max-w-prose font-mono text-xs leading-relaxed text-muted-foreground">
          Delivery is reliable first, not fast. A message can wait hours for a path to open, and that is normal on a mesh. The status always tells you where it is, and never claims more than the
          network confirmed.
        </p>
      </section>
    </div>
  );
}
