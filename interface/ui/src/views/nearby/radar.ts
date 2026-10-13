// Nearby copy and the ASCII radar placeholder (spec 6.4). Pure, so it is unit-tested. There are no
// peers here on purpose: Layer 7 (Relay & presence) is what will know who is around.
import type { Status } from "../../lib/api/types";

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

export const NODE_GLYPH = "@";
export const RING_GLYPH = "·";

/**
 * Concentric rings with this node at the centre. A character cell is about twice as tall as it is wide,
 * so columns span twice the rows. Each ring is walked row by row and column by column so it has no gaps.
 * Only rings and the node: never a peer.
 */
export function radarArt(radius = 8, rings = 3): string {
  const w = 4 * radius + 1;
  const h = 2 * radius + 1;
  const grid: string[][] = Array.from({ length: h }, () => Array.from({ length: w }, () => " "));
  const put = (x: number, y: number) => {
    const row = grid[y + radius];
    if (row && x + 2 * radius >= 0 && x + 2 * radius < w) row[x + 2 * radius] = RING_GLYPH;
  };
  for (let k = 1; k <= rings; k++) {
    const r = (radius * k) / rings;
    for (let y = -radius; y <= radius; y++) {
      if (Math.abs(y) > r) continue;
      const x = Math.round(2 * Math.sqrt(r * r - y * y));
      put(x, y);
      put(-x, y);
    }
    for (let x = -2 * radius; x <= 2 * radius; x++) {
      if (Math.abs(x) > 2 * r) continue;
      const y = Math.round(Math.sqrt(r * r - (x / 2) * (x / 2)));
      put(x, y);
      put(x, -y);
    }
  }
  grid[radius]![2 * radius] = NODE_GLYPH;
  return grid.map((row) => row.join("").replace(/\s+$/, "")).join("\n");
}
