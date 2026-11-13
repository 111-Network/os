// Nearby copy and the radar (os/7-relay/README.md section 10; interface spec 6.4). The radar is plain SVG (it draws on old phones) built from
// `radarModel` (model.ts): this file only turns that model into elements, with `createElement` because the file is `.ts`. Every label is
// text the model already cleaned and cut; nothing here is a link, a script or a style from a node.
import { createElement as h, type ReactElement } from "react";
import { ageLabel } from "../../lib/api/nearby";
import type { Status } from "../../lib/api/types";
import { cellSidePx, dotPoint, labelStep, RADAR_BOX, RADAR_LABEL_SIZE, ringName, ringRadius, type RadarDot, type RadarModel } from "./model";

/** One honest sentence from `status.transport_state`. Null (no status yet) says nothing is known. */
export function nearbySentence(state: Status["transport_state"] | null | undefined): string {
  switch (state) {
    case "joined":
      return "At least one nearby node is heard on the mesh.";
    case "isolated":
      return "No nearby nodes heard right now.";
    case "no transport":
      return "No mesh transport yet.";
    case "transport degraded":
      return "The mesh transport has a problem.";
    default:
      return "Waiting for the node to report what it hears.";
  }
}

// ------------------------------------------------------------------------------------------ the SVG radar

/** A dot's tooltip: its name, its ring in words and its age, all text. */
export function dotTitle(d: RadarDot, capped: boolean): string {
  return `${d.name} · ${ringName(d.ring, capped)}${d.ageS !== null ? ` · heard ${ageLabel(d.ageS)}` : ""}`;
}

/** One sentence for assistive technology: how many nodes are drawn and on how many rings. */
export function radarLabel(model: RadarModel): string {
  const n = model.dots.length;
  return `Radar: ${n === 0 ? "no nodes heard" : n === 1 ? "1 node heard" : `${n} nodes heard`}, this node in the centre, ${model.rings} rings of hops and an outer ring for unknown hops.`;
}

function glyph(d: RadarDot, x: number, y: number): ReactElement {
  const fx = (n: number): string => n.toFixed(1);
  const common = { fill: "currentColor", stroke: "none" };
  const mark =
    d.glyph === "triangle"
      ? h("polygon", { ...common, points: `${fx(x)},${fx(y - 5.5)} ${fx(x + 5)},${fx(y + 4)} ${fx(x - 5)},${fx(y + 4)}` })
      : d.glyph === "diamond"
        ? h("polygon", { ...common, points: `${fx(x)},${fx(y - 5.5)} ${fx(x + 5.5)},${fx(y)} ${fx(x)},${fx(y + 5.5)} ${fx(x - 5.5)},${fx(y)}` })
        : d.glyph === "circle"
          ? h("circle", { ...common, cx: fx(x), cy: fx(y), r: 4.5 })
          : h("rect", { ...common, x: fx(x - 4), y: fx(y - 4), width: 8, height: 8 });
  return mark;
}

/**
 * The radar: this node in the centre, one ring band per hop (ring 1 is heard directly), a dashed outer band for unknown hops, a shaded
 * square under each Meshtastic position (the coarsest cell in v1), a mark per node by its source. Direction is the bearing only when both
 * ends shared a position (the model decides); the legend that says so is the caller's, verbatim.
 */
export function RadarSvg({ model }: { model: RadarModel }): ReactElement {
  const { size, centre } = RADAR_BOX;
  const R = model.rings;
  const bands: ReactElement[] = [];
  // Ring names are drawn last, over the dots, each with a halo in the card's colour: a dot or a ring line under a name
  // never makes it unreadable (the innermost names are wider than their rings).
  const names: ReactElement[] = [];
  const step = labelStep(R);
  for (let j = 1; j <= R + 1; j++) {
    const outer = j === R + 1;
    const r = ringRadius(j, R);
    bands.push(h("circle", { key: `ring-${j}`, cx: centre, cy: centre, r, fill: "none", stroke: "currentColor", strokeWidth: 1, strokeDasharray: outer ? "4 3" : undefined, className: "text-border" }));
    if ((R + 1 - j) % step === 0) {
      names.push(
        h(
          "text",
          { key: `name-${j}`, "data-testid": "radar-ring-label", x: centre, y: centre - r + 4, textAnchor: "middle", fontSize: RADAR_LABEL_SIZE, fill: "currentColor", stroke: "var(--card)", strokeWidth: 3, strokeLinejoin: "miter", paintOrder: "stroke", className: "text-muted-foreground" },
          outer ? ringName(null) : ringName(j, model.capped),
        ),
      );
    }
  }
  const cells: ReactElement[] = [];
  const marks: ReactElement[] = [];
  for (const d of model.dots) {
    const { x, y } = dotPoint(d, R);
    if (d.cell) {
      const side = cellSidePx(d.cell.km);
      cells.push(
        h("rect", {
          key: `cell-${d.key}`,
          "data-testid": "radar-cell",
          x: (x - side / 2).toFixed(1),
          y: (y - side / 2).toFixed(1),
          width: side,
          height: side,
          fill: "currentColor",
          fillOpacity: 0.14,
          stroke: "currentColor",
          strokeOpacity: 0.4,
          strokeWidth: 0.75,
          className: "text-foreground",
        }),
      );
    }
    marks.push(
      h(
        "g",
        { key: d.key, className: "text-foreground", "data-testid": "radar-dot", "data-kind": d.kind, "data-ring": d.ring === null ? "unplaced" : String(d.ring) },
        h("title", null, dotTitle(d, model.capped)),
        glyph(d, x, y),
      ),
    );
  }
  return h(
    "svg",
    { viewBox: `0 0 ${size} ${size}`, role: "img", "aria-label": radarLabel(model), className: "h-auto w-full max-w-md", "data-testid": "radar" },
    ...bands,
    ...cells,
    h("circle", { key: "me", cx: centre, cy: centre, r: 4, fill: "currentColor", className: "text-foreground" }),
    h("circle", { key: "me-ring", cx: centre, cy: centre, r: 7, fill: "none", stroke: "currentColor", strokeWidth: 1, className: "text-foreground" }),
    ...marks,
    ...names,
  );
}
