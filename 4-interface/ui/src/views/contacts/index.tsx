// Contacts (os/5-messaging/README.md section 16, at /messages/contacts): message requests, the contact list, add by card /
// QR / node id, and this node's own card.
import { hrefTo } from "../../app/location";
import { ButtonLink } from "../../components/button";
import { Empty } from "../../components/empty";
import { SectionHeader } from "../../components/section-header";
import { useApiMeta, useContacts, useThreads } from "../../lib/api/hooks";
import { openRequests } from "../../lib/api/contacts";
import { requestPreview } from "../messages/model";
import { requestSeq } from "../../lib/api/requests";
import { RequestsSection } from "../messages/requests";
import { AddContact } from "./add";
import { ContactList } from "./list";
import { sortedContacts } from "./model";
import { OwnCard } from "./own-card";

export default function ContactsView() {
  const contacts = useContacts();
  const threads = useThreads();
  const { ready } = useApiMeta();

  const back = (
    <ButtonLink href={hrefTo("/messages")} size="sm">
      All messages
    </ButtonLink>
  );

  // A request's newest receipt comes from its thread; without the thread it is shown but cannot be ignored.
  const requests = openRequests(contacts).map((c) => {
    const t = threads?.find((x) => x.peer === c.node_id) ?? null;
    return { peer: c.node_id, label: c.label, preview: t ? requestPreview(t) : null, seq: t ? requestSeq(t) : null };
  });

  return (
    <div className="flex flex-col gap-6">
      <SectionHeader kicker="messages" title="Contacts" level={1} end={back} />
      <div className="grid gap-6 lg:grid-cols-[minmax(0,1fr)_minmax(0,24rem)]">
        <div className="flex min-w-0 flex-col gap-6">
          <RequestsSection items={requests} headingId="contact-requests-h" />
          <section aria-labelledby="contact-list-h" className="flex flex-col gap-2">
            <h2 id="contact-list-h" className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground">
              contacts
            </h2>
            {contacts === null ? (
              ready ? (
                <Empty kicker="unavailable" title="Contacts could not be loaded">
                  The node did not answer. Your contacts appear here as soon as it does.
                </Empty>
              ) : (
                <p role="status" className="font-mono text-xs text-muted-foreground">
                  Loading contacts
                </p>
              )
            ) : (
              <ContactList contacts={sortedContacts(contacts)} />
            )}
          </section>
        </div>
        <div className="flex min-w-0 flex-col gap-6">
          <AddContact />
          <OwnCard />
        </div>
      </div>
    </div>
  );
}
