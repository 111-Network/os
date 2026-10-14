// The store gauge (os/messaging/README.md section 10): live bytes against the high watermark, as a bar and as words. Colour
// is never used for it (DESIGN.md: colour lives on status dots only); a full store is said in words and by the service's
// ERROR dot, and the bar is solid when there is no room left.
import type { FabricStore } from "../../lib/api/types";
import { storePercent } from "../../lib/api/fabric";
import { storeUsageText } from "./fabric-model";

export function StoreGauge({ store, label = "Store used" }: { store: Pick<FabricStore, "used" | "watermark" | "open" | "store_full">; label?: string }) {
  const pct = store.store_full ? 100 : storePercent(store);
  return (
    <div className="flex flex-col gap-2" data-testid="store-gauge">
      <div
        role="meter"
        aria-label={label}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={pct}
        aria-valuetext={`${pct} percent`}
        className="h-3 w-full border border-border"
      >
        <div className="h-full bg-foreground" style={{ width: `${pct}%` }} />
      </div>
      <p className="font-mono text-xs leading-relaxed text-muted-foreground">
        <span className="text-foreground">{pct}%</span> {" · "}
        {store.open ? storeUsageText(store.used, store.watermark) : "not open"}
      </p>
    </div>
  );
}
