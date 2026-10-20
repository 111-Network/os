// A QR code drawn from the module matrix the node sends (os/5-messaging/README.md section 7.2): dark modules on a light plate in
// the page's own tokens (the dashboard is dark, so the plate is `foreground`). Used for the hub's enrollment bundle.
import { useEffect, useMemo, useRef } from "react";
import { drawQr, QR_QUIET_ZONE, qrModules, qrScale } from "../../lib/api/contacts";
import type { Card } from "../../lib/api/types";

const TARGET_PX = 320;

function plateColors(): { dark: string; light: string } {
  const cs = getComputedStyle(document.documentElement);
  return { dark: cs.getPropertyValue("--background").trim() || "#000000", light: cs.getPropertyValue("--foreground").trim() || "#ffffff" };
}

export function QrCanvas({ qr, label }: { qr: Card["qr"]; label: string }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const modules = useMemo(() => qrModules(qr), [qr]);

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

  if (!modules) return <p className="font-mono text-xs text-muted-foreground">The QR code could not be drawn. The text below still works.</p>;
  return <canvas ref={canvas} role="img" aria-label={label} className="aspect-square w-full max-w-xs border border-border [image-rendering:pixelated]" />;
}
