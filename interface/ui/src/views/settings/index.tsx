// Settings (spec section 6.7). The router sends `/settings` and `/settings/:layer` here:
// no param = Essentials, `l0`..`l9` = a layer page, anything else = 404.
import { useParams } from "wouter";
import { Empty } from "../../components/empty";
import { ButtonLink } from "../../components/button";
import { layerBySlug } from "../../lib/layers";
import { hrefTo } from "../../app/location";
import { Essentials } from "./essentials";
import { LayerPage } from "./layer-page";

export default function SettingsView() {
  const { layer } = useParams<{ layer?: string }>();
  if (layer === undefined) return <Essentials />;
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
