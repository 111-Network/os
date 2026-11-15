// Settings, Relay & presence, Discovery (os/7-relay/README.md §7.1, §7.2, §12): the LAN record and the role bits 3 to 5, each with
// what it costs. Bit 3 asks for a confirmation (`gateway_bit`: the node's exact words); bit 5 is the update-serving switch below.
import { Panel } from "../../../components/panel";
import type { RelayStatus } from "../../../lib/api/types";
import { FEEDBACK, Row } from "../row";
import { DISCOVERY_BITS, HUB_ONLY_TEXT, PATCH } from "./model";
import { SwitchRow } from "./switch";

export function DiscoverySection({ relay, hubGate }: { relay: RelayStatus; hubGate: boolean }) {
  const d = relay.discovery;
  const cost = (id: string) => DISCOVERY_BITS.find((b) => b.id === id)!;
  return (
    <Panel title="discovery" suffix="who can find this node" flush>
      <SwitchRow id="lan-record" label={cost("lan_record").label} help={cost("lan_record").cost} on={d.lan_record} patch={PATCH.lanRecord} />
      <SwitchRow
        id="gateway-bit"
        label={cost("public_gateway_bit").label}
        help={hubGate ? `${cost("public_gateway_bit").cost} ${HUB_ONLY_TEXT}` : cost("public_gateway_bit").cost}
        on={d.public_gateway_bit}
        disabled={hubGate}
        patch={PATCH.gatewayBit}
      />
      <SwitchRow id="offload-bit" label={cost("offload_bit").label} help={cost("offload_bit").cost} on={d.offload_bit} patch={PATCH.offloadBit} />
      <Row id="origin-bit" label={cost("origin").label} help={cost("origin").cost}>
        <p className={`text-foreground ${FEEDBACK}`} data-testid="origin-bit-state">
          {relay.origin.serve ? "on, because this hub serves update files" : "off"}
        </p>
      </Row>
    </Panel>
  );
}
