// Renders a proposal's markdown body from the AST of src/lib/markdown.ts. The parser never produces
// HTML, so every character reaches the page as an escaped text node; links pass its allowlist.
import { useMemo, type ReactNode } from "react";
import { hrefTo } from "../../app/location";
import { kbRoutePath, parseMarkdown, type Block, type Inline } from "../../lib/markdown";

const LINK = "underline underline-offset-4 hover:text-foreground focus-visible:ring-2 focus-visible:ring-foreground focus-visible:outline-none";

function inline(nodes: readonly Inline[]): ReactNode {
  return nodes.map((n, i) => {
    switch (n.t) {
      case "text":
        return n.text;
      case "code":
        return (
          <code key={i} className="bg-secondary px-1 text-foreground">
            {n.text}
          </code>
        );
      case "strong":
        return (
          <strong key={i} className="font-bold text-foreground">
            {inline(n.children)}
          </strong>
        );
      case "em":
        return <em key={i}>{inline(n.children)}</em>;
      case "link":
        return n.link.kind === "external" ? (
          <a key={i} href={n.link.href} target="_blank" rel="noopener noreferrer" className={LINK}>
            {inline(n.children)}
            <span className="sr-only"> (opens in a new tab)</span>
          </a>
        ) : (
          <a key={i} href={hrefTo(kbRoutePath(n.link.path))} className={LINK}>
            {inline(n.children)}
          </a>
        );
    }
  });
}

function blocks(list: readonly Block[]): ReactNode {
  return list.map((b, i) => {
    switch (b.t) {
      case "heading":
        return (
          <p key={i} className="mt-2 font-bold text-foreground">
            {inline(b.children)}
          </p>
        );
      case "paragraph":
        return (
          <p key={i} className="mt-2 break-words">
            {inline(b.children)}
          </p>
        );
      case "list": {
        const Tag = b.ordered ? "ol" : "ul";
        return (
          <Tag key={i} {...(b.ordered ? { start: b.start } : {})} className={`mt-2 ps-5 ${b.ordered ? "list-decimal" : "list-disc"}`}>
            {b.items.map((it, j) => (
              <li key={j} className="break-words">
                {inline(it.children)}
                {it.sub.length ? blocks(it.sub) : null}
              </li>
            ))}
          </Tag>
        );
      }
      case "code":
        return (
          <pre key={i} tabIndex={0} className="mt-2 overflow-x-auto border border-border bg-background p-2 text-foreground focus-visible:ring-2 focus-visible:ring-foreground focus-visible:outline-none">
            {b.text}
          </pre>
        );
      case "quote":
        return (
          <blockquote key={i} className="mt-2 border-s border-border ps-3">
            {blocks(b.children)}
          </blockquote>
        );
      case "rule":
        return <hr key={i} className="mt-2 border-border" />;
    }
  });
}

export function ProposalBody({ body }: { body: string }) {
  const doc = useMemo(() => parseMarkdown(body), [body]);
  return (
    <div className="max-h-72 overflow-auto border border-border bg-secondary/20 p-3 pt-1 text-xs leading-relaxed text-muted-foreground">
      {blocks(doc.blocks)}
      {doc.truncated ? <p className="mt-2">[truncated]</p> : null}
    </div>
  );
}
