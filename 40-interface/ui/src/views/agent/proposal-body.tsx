// Renders a proposal's markdown body from the AST of src/lib/markdown.ts, with the same renderer the
// knowledge base uses (headings as bold lines here, so a proposal never adds to the page outline). The
// parser never produces HTML, so every character reaches the page as an escaped text node; links pass its allowlist.
import { useMemo } from "react";
import { parseMarkdown } from "../../lib/markdown";
import { Blocks } from "../kb/render";

export function ProposalBody({ body }: { body: string }) {
  const doc = useMemo(() => parseMarkdown(body), [body]);
  return (
    <div className="max-h-72 overflow-auto border border-border bg-secondary/20 p-3 pt-1">
      <Blocks blocks={doc.blocks} headingsAsBold compact />
      {doc.truncated ? <p className="mt-2 text-xs text-muted-foreground">[truncated]</p> : null}
    </div>
  );
}
