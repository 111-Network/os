// This node's own card (os/50-messaging/README.md section 7.2): the public announce as a QR the page draws itself from
// the module bitmap, and as text with a copy button. Public material only: it holds no secret.
import { useEffect, useMemo, useRef } from "react";
import { CopyButton } from "../../components/copy-button";
import { Panel } from "../../components/panel";
import { useOwnCard, useStatus } from "../../lib/api/hooks";
import { drawQr, QR_QUIET_ZONE, qrModules, qrScale } from "../../lib/api/contacts";
import { Adv } from "../../lib/mode";

const TARGET_PX = 320;

/** Dark modules on a light plate in the page's own tokens (the dashboard is dark: the plate is `foreground`). */
function plateColors(): { dark: string; light: string } {
  const cs = getComputedStyle(document.documentElement);
  return { dark: cs.getPropertyValue("--background").trim() || "#000000", light: cs.getPropertyValue("--foreground").trim() || "#ffffff" };
}

export function OwnCard() {
  const card = useOwnCard();
  const status = useStatus();
  const canvas = useRef<HTMLCanvasElement>(null);
  const modules = useMemo(() => (card ? qrModules(card.qr) : null), [card]);

  useEffect(() => {
    const el = canvas.current;
    const ctx = el?.getContext("2d");
    if (!el || !ctx || !modules) return;
    const scale = qrScale(modules.length, TARGET_PX);
    const side = (modules.length + 2 * QR_QUIET_ZONE) * scale;
    el.width = side;
    el.height = side;
    const { dark, light } = plateColors();
    drawQr(ctx, modules, scale, dark, light);
  }, [modules]);

  const nodeId = status?.node.bech32 ?? null;

  return (
    <Panel title="your card" suffix="public" heading={2}>
      <div className="flex flex-col gap-4 font-mono">
        <p className="max-w-prose text-xs leading-relaxed text-muted-foreground">
          Show this to someone, or send the text. It is public: it lets them write to you and holds no secret.
        </p>
        {card === null ? (
          <p role="status" className="text-xs text-muted-foreground">
            Loading your card
          </p>
        ) : (
          <>
            {modules ? (
              <canvas ref={canvas} role="img" aria-label="Your contact card as a QR code" className="aspect-square w-full max-w-xs border border-border [image-rendering:pixelated]" />
            ) : (
              <p className="text-xs text-muted-foreground">The QR code could not be drawn. The card text below still works.</p>
            )}
            <div className="flex flex-col gap-2">
              <span className="text-[10px] uppercase tracking-widest text-muted-foreground">card text</span>
              <p className="max-h-32 overflow-y-auto break-all border border-border p-2 text-[11px] leading-snug text-foreground" tabIndex={0} aria-label="Your card text">
                {card.text}
              </p>
              <div>
                <CopyButton text={card.text} label="copy card" ariaLabel="Copy your card text" />
              </div>
            </div>
          </>
        )}
        {nodeId ? (
          <div className="flex flex-col gap-2">
            <span className="text-[10px] uppercase tracking-widest text-muted-foreground">your node id</span>
            <p className="break-all text-[11px] leading-snug text-foreground">{nodeId}</p>
            <div>
              <CopyButton text={nodeId} label="copy id" ariaLabel="Copy your node id" />
            </div>
          </div>
        ) : null}
        <Adv>
          <p className="text-[11px] leading-snug text-muted-foreground">
            {card ? `QR ${card.qr.size}×${card.qr.size} modules, ${card.text.length} characters of text.` : ""}
          </p>
        </Adv>
      </div>
    </Panel>
  );
}
