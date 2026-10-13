// Renders the markdown AST (lib/markdown.ts) as React elements. Every string reaches the page as an
// escaped text node: there is no innerHTML anywhere, and links have already passed the allowlist.
import type { ReactNode } from "react";
import { hrefTo } from "../../app/location";
import { CopyButton } from "../../components/copy-button";
import { cx, FOCUS_RING } from "../../lib/cx";
import { kbRoutePath, type Block, type Inline } from "../../lib/markdown";
import { LINK_CLASS } from "../services/shared";

function InlineNode({ node }: { node: Inline }): ReactNode {
  switch (node.t) {
    case "text":
      return node.text;
    case "code":
      return <code className="bg-secondary px-1 py-0.5 text-[0.85em] text-foreground">{node.text}</code>;
    case "strong":
      return (
        <strong className="font-bold text-foreground">
          <Inlines nodes={node.children} />
        </strong>
      );
    case "em":
      return (
        <em>
          <Inlines nodes={node.children} />
        </em>
      );
    case "link":
      return node.link.kind === "kb" ? (
        <a href={hrefTo(kbRoutePath(node.link.path))} className={LINK_CLASS}>
          <Inlines nodes={node.children} />
        </a>
      ) : (
        <a href={node.link.href} target="_blank" rel="noopener noreferrer" className={LINK_CLASS}>
          <Inlines nodes={node.children} />
          <span className="sr-only"> (opens in a new tab)</span>
        </a>
      );
  }
}

export function Inlines({ nodes }: { nodes: readonly Inline[] }) {
  return (
    <>
      {nodes.map((n, i) => (
        <InlineNode key={i} node={n} />
      ))}
    </>
  );
}

function CodeBlock({ lang, text }: { lang: string; text: string }) {
  return (
    <div className="border border-border">
      <div className="flex items-center justify-between gap-3 border-b border-border px-3 py-1">
        <span className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground">{lang || "code"}</span>
        <CopyButton text={text} label="copy" ariaLabel={`Copy ${lang || "code"} block`} />
      </div>
      <pre
        tabIndex={0}
        className={cx("max-h-96 overflow-auto bg-secondary/20 p-3 font-mono text-xs leading-relaxed text-foreground focus-visible:ring-inset", FOCUS_RING)}
      >
        <code>{text}</code>
      </pre>
    </div>
  );
}

const HEADING_TAG = { 1: "h2", 2: "h3", 3: "h4", 4: "h5", 5: "h6", 6: "h6" } as const;
const HEADING_CLASS = {
  1: "mt-8 font-pixel-line text-xl text-foreground",
  2: "mt-6 font-mono text-sm font-bold uppercase tracking-widest text-foreground",
  3: "mt-4 font-mono text-sm font-bold text-foreground",
  4: "mt-4 font-mono text-xs font-bold text-foreground",
  5: "mt-4 font-mono text-xs font-bold text-muted-foreground",
  6: "mt-4 font-mono text-xs font-bold text-muted-foreground",
} as const;

export interface RenderOptions {
  /**
   * Headings are bold lines, not document headings: for a body shown inside another page (a proposal
   * in the chat) where it must not add to the outline.
   */
  headingsAsBold?: boolean;
  /** Small muted text for a body inside a card. */
  compact?: boolean;
  /**
   * The shallowest heading level in the document (see `minHeadingLevel`). Headings are numbered from it,
   * so the first one is an h2 under the page's h1 whatever level the author started at.
   */
  minLevel?: number;
}

/** The shallowest heading level among the top-level blocks (1 when there is none). */
export function minHeadingLevel(blocks: readonly Block[]): number {
  let min = 7;
  for (const b of blocks) if (b.t === "heading" && b.level < min) min = b.level;
  return min === 7 ? 1 : min;
}

function BlockNode({ block, opts }: { block: Block; opts: RenderOptions }): ReactNode {
  const small = opts.compact === true;
  const text = small ? "break-words text-xs leading-relaxed text-muted-foreground" : "font-mono text-sm leading-relaxed text-foreground";
  switch (block.t) {
    case "heading": {
      if (opts.headingsAsBold) {
        return (
          <p className="mt-2 font-bold text-foreground">
            <Inlines nodes={block.children} />
          </p>
        );
      }
      // The page title is the h1, so document headings start at h2 and never skip a level.
      const rel = Math.min(6, Math.max(1, block.level - (opts.minLevel ?? 1) + 1)) as 1 | 2 | 3 | 4 | 5 | 6;
      const Tag = HEADING_TAG[rel];
      return (
        <Tag className={HEADING_CLASS[block.level]}>
          <Inlines nodes={block.children} />
        </Tag>
      );
    }
    case "paragraph":
      return (
        <p className={text}>
          <Inlines nodes={block.children} />
        </p>
      );
    case "list": {
      const Tag = block.ordered ? "ol" : "ul";
      return (
        <Tag start={block.ordered ? block.start : undefined} className={cx("space-y-1 ps-6", text, block.ordered ? "list-decimal" : "list-disc")}>
          {block.items.map((item, i) => (
            <li key={i}>
              <Inlines nodes={item.children} />
              {item.sub.length > 0 ? (
                <div className="mt-1">
                  <Blocks blocks={item.sub} {...opts} />
                </div>
              ) : null}
            </li>
          ))}
        </Tag>
      );
    }
    case "code":
      return <CodeBlock lang={block.lang} text={block.text} />;
    case "quote":
      return (
        <blockquote className="border-s-2 border-border ps-4 text-muted-foreground">
          <Blocks blocks={block.children} {...opts} />
        </blockquote>
      );
    case "rule":
      return <hr className="border-border" />;
  }
}

export function Blocks({ blocks, ...opts }: { blocks: readonly Block[] } & RenderOptions) {
  return (
    <div className={opts.compact ? "space-y-2" : "space-y-3"}>
      {blocks.map((b, i) => (
        <BlockNode key={i} block={b} opts={opts} />
      ))}
    </div>
  );
}
