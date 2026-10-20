// What the Messaging & data page asks the node to do, as plain functions over an adapter and the store: a change is a call the
// node answers, then a fresh read of what changed. The page never shows a value before the node has answered (no optimistic
// updates); the same read also arrives as a `fabric` frame. Tested against the mock adapter.
import type { Adapter } from "../../lib/api/adapter";
import type { ApiStore } from "../../lib/api/store";
import type { FabricSettings, FabricSettingsPatch, HubEnrollRequest, HubOffer } from "../../lib/api/types";
import type { SettingKey } from "./messaging-model";

/** Enroll with a hub (member side). Resolves when the node enrolled this one; rejects with the node's problem otherwise. */
export async function enrollWithHub(adapter: Adapter, store: ApiStore, body: HubEnrollRequest): Promise<void> {
  await adapter.fabric.enrollHub(body);
  // The hub is now a contact, the `fabric` row may have changed: read all three rather than guess.
  await Promise.all([store.refreshFabric(), store.refreshContacts(), store.refreshServices()]);
}

/** Change one setting. Resolves with the value the node answered. */
export async function changeSetting(adapter: Adapter, store: ApiStore, field: SettingKey, next: boolean): Promise<FabricSettings> {
  const patch: FabricSettingsPatch = { [field]: next };
  const answered = await adapter.fabric.patchSettings(patch);
  await store.refreshFabric();
  return answered;
}

/** Save the changed quotas (`null` drops one). */
export async function changeQuotas(adapter: Adapter, store: ApiStore, quotas: Record<string, number | null>): Promise<FabricSettings> {
  const answered = await adapter.fabric.patchSettings({ quotas });
  await store.refreshFabric();
  return answered;
}

/** Mint a one-time code with the hub's card and address as one bundle (loopback-only). The code is in this reply and nowhere else. */
export async function mintOffer(adapter: Adapter, store: ApiStore): Promise<HubOffer> {
  const offer = await adapter.fabric.offerHub();
  await store.refreshFabric();
  return offer;
}

/** Dismiss the outstanding code (loopback-only; idempotent). */
export async function dismissOffer(adapter: Adapter, store: ApiStore): Promise<void> {
  await adapter.fabric.dismissHubOffer();
  await store.refreshFabric();
}
