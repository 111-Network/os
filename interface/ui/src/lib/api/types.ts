// Single import point for contract types. Generated files are never edited by hand:
//   types.gen.ts  openapi-typescript output (paths, components, operations)
//   sse.gen.ts    SSE frame unions + schema aliases + enum tuples (scripts/gen-sse.mjs)
import type { DeviceRecord } from "./sse.gen";

export type { components, operations, paths } from "./types.gen";
export * from "./sse.gen";

/** The offer a board carries when it has no usable firmware (`DeviceRecord.flash_offer`, Layer 6, D17). */
export type FlashOffer = NonNullable<DeviceRecord["flash_offer"]>;
