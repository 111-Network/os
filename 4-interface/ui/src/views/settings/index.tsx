// Settings (spec section 6.7). The router sends `/settings` and `/settings/:layer` here:
// no param = Essentials, `l5` = Messaging & data and `l6` = Transport (real controls), the other `l0`..`l9` = a layer page, anything else = 404.
import { useParams } from "wouter";
import { Empty } from "../../components/empty";
import { ButtonLink } from "../../components/button";
import { layerBySlug } from "../../lib/layers";
import { hrefTo } from "../../app/location";
import { lazy } from "react";
import { Essentials } from "./essentials";
import { LayerPage } from "./layer-page";
import { MessagingSettings } from "./messaging";

// Layer 6 has its own lazy chunk (D21: at most 64 KB gzip).
const TransportSettings = lazy(() => import("./l6"));

export default function SettingsView() {
  const { layer } = useParams<{ layer?: string }>();
  if (layer === undefined) return <Essentials />;
  if (layer === "l6") return <TransportSettings />; // Layer 6: adapters, order, posture, region, relays, lists, components, confirmations (l6/)
  if (layer === "l5") return <MessagingSettings />; // Layer 5 has real rows (messaging.tsx); the other layer pages are structure only
  const info = layerBySlug(layer);
  if (!info) {
    return (
      <Empty
        kicker="404"
        title="No such layer"
        action={
          <ButtonLink href={hrefTo("/settings")} size="sm">
            Back to Essentials
          </ButtonLink>
        }
      >
        There is no settings page for “{layer}”. Layers are l0 to l9.
      </Empty>
    );
  }
  return <LayerPage layer={info} />;
}
