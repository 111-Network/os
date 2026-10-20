// The lazy chunk behind /messages/net/:network[/:conv] and /messages/send (os/6-transport/README.md §10, D21 cap 64 KB gzip). The router
// places these before /messages/:peer; this view branches on the params.
import { useParams } from "wouter";
import { paramText } from "../../../lib/api/networks";
import FanoutView from "./fanout";
import NetworkPage from "./network";
import ForeignThreadView from "./thread";

export default function NetView() {
  const params = useParams<{ network?: string; conv?: string }>();
  const network = paramText(params.network);
  const conv = paramText(params.conv);
  if (network === undefined) return <FanoutView />;
  if (conv === undefined) return <NetworkPage key={network} network={network} />;
  return <ForeignThreadView key={`${network}/${conv}`} network={network} conv={conv} />;
}
