// One on/off setting of the Layer 7 page: a label with its cost, a toggle, and the consent round-trip when the node asks for one.
import type { ReactNode } from "react";
import { Toggle } from "../../../components/toggle";
import type { RelaySettingsPatch } from "../../../lib/api/types";
import { Row } from "../row";
import { useRelayCall } from "./actions";
import { CallOutput } from "./call-view";

export interface SwitchRowProps {
  id: string;
  label: string;
  help: ReactNode;
  on: boolean;
  /** The setting is not available here (no hub role): the toggle is disabled and `help` says why. */
  disabled?: boolean;
  /** The settings write for the new value. */
  patch(next: boolean): RelaySettingsPatch;
  /** Polite line after the call went through. */
  done?(next: boolean): string;
}

export function SwitchRow({ id, label, help, on, disabled = false, patch, done }: SwitchRowProps) {
  const call = useRelayCall();
  const press = (next: boolean) => void call.patch(patch(next), { done: done?.(next) ?? (next ? `${label} on.` : `${label} off.`) });
  return (
    <Row id={id} label={label} help={help}>
      <Toggle pressed={on} disabled={disabled} busy={call.state.pending} onPressedChange={press}>
        <span>{label}</span> <span aria-hidden="true">{on ? "[ON]" : "[OFF]"}</span>
      </Toggle>
      <CallOutput call={call} />
    </Row>
  );
}
