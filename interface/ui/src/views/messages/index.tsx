// Messages (os/messaging/README.md section 16). The router sends /messages and /messages/:peer here; this view
// branches on the route. /messages/contacts is its own view.
import { useParams } from "wouter";
import { ButtonLink } from "../../components/button";
import { Empty } from "../../components/empty";
import { hrefTo } from "../../app/location";
import ThreadList from "./list";
import { peerFromParam } from "./model";
import ThreadView from "./thread";

export default function MessagesView() {
  const { peer } = useParams<{ peer?: string }>();
  if (peer === undefined) return <ThreadList />;
  const id = peerFromParam(peer);
  if (!id) {
    return (
      <Empty
        kicker="not found"
        title="No such conversation"
        action={
          <ButtonLink href={hrefTo("/messages")} size="sm">
            All messages
          </ButtonLink>
        }
      >
        That address is not a node id.
      </Empty>
    );
  }
  return <ThreadView key={id} peer={id} />;
}
