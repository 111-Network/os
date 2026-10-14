// The camera scanner: a live preview, one decode attempt every SCAN_INTERVAL_MS, and the camera released the moment a
// code is read, the dialog is closed or the view goes away. Offered only where `cameraSupport` says so.
import { useEffect, useRef, useState } from "react";
import { Button } from "../../components/button";
import { cameraErrorText, decodeFrame, fitScale, loadDecoder, SCAN_INTERVAL_MS } from "./scan";

export function Scanner({ onText, onClose }: { onText: (text: string) => void; onClose: () => void }) {
  const video = useRef<HTMLVideoElement>(null);
  const [status, setStatus] = useState("Starting the camera");
  const [failed, setFailed] = useState(false);
  // The callbacks may change identity on every render of the parent; the effect must not restart the camera for that.
  const cb = useRef({ onText, onClose });
  cb.current = { onText, onClose };

  useEffect(() => {
    let stopped = false;
    let stream: MediaStream | null = null;
    let raf = 0;
    const stop = () => {
      stopped = true;
      cancelAnimationFrame(raf);
      stream?.getTracks().forEach((t) => t.stop());
      stream = null;
      if (video.current) video.current.srcObject = null;
    };
    void (async () => {
      try {
        const [decode, media] = await Promise.all([
          loadDecoder(),
          navigator.mediaDevices.getUserMedia({ video: { facingMode: "environment" }, audio: false }),
        ]);
        if (stopped) {
          media.getTracks().forEach((t) => t.stop());
          return;
        }
        stream = media;
        const el = video.current;
        if (!el) return stop();
        el.srcObject = media;
        await el.play();
        setStatus("Looking for a QR code. Hold it steady in view.");
        const canvas = document.createElement("canvas");
        const ctx = canvas.getContext("2d", { willReadFrequently: true });
        let last = 0;
        const tick = (t: number) => {
          if (stopped) return;
          raf = requestAnimationFrame(tick);
          if (t - last < SCAN_INTERVAL_MS || !ctx || el.videoWidth === 0) return;
          last = t;
          const k = fitScale(el.videoWidth, el.videoHeight, 640);
          const w = Math.round(el.videoWidth * k);
          const h = Math.round(el.videoHeight * k);
          canvas.width = w;
          canvas.height = h;
          ctx.drawImage(el, 0, 0, w, h);
          const text = decodeFrame(decode, ctx.getImageData(0, 0, w, h).data, w, h);
          if (text !== null && text.trim() !== "") {
            stop();
            cb.current.onText(text);
          }
        };
        raf = requestAnimationFrame(tick);
      } catch (e) {
        stop();
        setFailed(true);
        setStatus(e instanceof DOMException ? cameraErrorText(e.name) : cameraErrorText(undefined));
      }
    })();
    return stop;
  }, []);

  return (
    <div className="flex flex-col gap-3 border border-border p-3">
      <video ref={video} muted playsInline aria-label="Camera preview" className="aspect-square w-full max-w-xs border border-border bg-secondary object-cover" hidden={failed} />
      <p role="status" className="max-w-prose font-mono text-xs leading-relaxed text-muted-foreground">
        {status}
      </p>
      <div>
        <Button size="sm" onClick={() => cb.current.onClose()}>
          {failed ? "close" : "cancel scanning"}
        </Button>
      </div>
    </div>
  );
}
