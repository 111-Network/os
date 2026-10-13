// Knowledge-base viewer (`/kb` and `/kb/<path>`): the node's offline recipes, read through the adapter
// and drawn by the tiny safe renderer in lib/markdown.ts.
import { useEffect, useMemo, useState } from "react";
import { redirectIfUnauthenticated } from "../../app/AuthGate";
import { hrefTo, useAppLocation } from "../../app/location";
import { Button, ButtonLink } from "../../components/button";
import { Empty } from "../../components/empty";
import { SectionHeader } from "../../components/section-header";
import { SpecTable, type SpecRow } from "../../components/spec-table";
import { getAdapter, isApiError } from "../../lib/api";
import { useMode } from "../../lib/mode";
import { buildSlugIndex, classifyHref, frontmatterRows, kbRouteTarget, parseMarkdown, type KbRoute } from "../../lib/markdown";
import { RouteLink } from "../services/shared";
import { Blocks } from "./render";

const TEXT = "font-mono text-xs leading-relaxed text-muted-foreground";

type Load =
  | { state: "loading" }
  | { state: "ok"; text: string }
  | { state: "error"; kind: "not_found" | "bad_request" | "unavailable"; message: string };

function useKbText(target: KbRoute) {
  const [load, setLoad] = useState<Load>({ state: "loading" });
  const [attempt, setAttempt] = useState(0);
  const key = target.kind === "entry" ? `entry:${target.path}` : target.kind;

  useEffect(() => {
    if (target.kind === "invalid") return;
    let alive = true;
    setLoad({ state: "loading" });
    void (async () => {
      try {
        const adapter = await getAdapter();
        const text = target.kind === "root" ? await adapter.kb.index() : await adapter.kb.entry(target.path);
        if (alive) setLoad({ state: "ok", text });
      } catch (err) {
        if (!alive || redirectIfUnauthenticated(err)) return;
        const code = isApiError(err) ? err.code : "internal";
        setLoad({
          state: "error",
          kind: code === "not_found" ? "not_found" : code === "bad_request" ? "bad_request" : "unavailable",
          message: isApiError(err) ? err.message : "The node did not answer.",
        });
      }
    })();
    return () => {
      alive = false;
    };
    // `target` is rebuilt on every render; `key` is what identifies it.
  }, [key, attempt]);

  return { load, retry: () => setAttempt((n) => n + 1) };
}

let slugIndex: Map<string, string> | null = null;

/** `[[slug]]` needs the index to find the folder. Fetched once, only for documents that use wikilinks. */
function useSlugIndex(enabled: boolean): Map<string, string> | null {
  const [index, setIndex] = useState<Map<string, string> | null>(slugIndex);
  useEffect(() => {
    if (!enabled || slugIndex) return;
    let alive = true;
    void getAdapter()
      .then((a) => a.kb.index())
      .then((text) => {
        slugIndex = buildSlugIndex(text);
        if (alive) setIndex(slugIndex);
      })
      .catch(() => undefined); // best effort: links fall back to <slug>.md
    return () => {
      alive = false;
    };
  }, [enabled]);
  return index;
}

function valueNode(value: string) {
  const link = classifyHref(value);
  return link?.kind === "external" ? (
    <a href={link.href} target="_blank" rel="noopener noreferrer" className="break-all underline underline-offset-4 hover:bg-foreground hover:text-background">
      {value}
      <span className="sr-only"> (opens in a new tab)</span>
    </a>
  ) : (
    value
  );
}

function Document({ text, path }: { text: string; path: string }) {
  const { mode } = useMode();
  const usesWiki = text.includes("[[");
  const slugs = useSlugIndex(usesWiki);
  const doc = useMemo(() => parseMarkdown(text, { basePath: path, resolveWiki: (slug) => slugs?.get(slug) ?? null }), [text, path, slugs]);
  const rows: SpecRow[] = doc.frontmatter
    ? frontmatterRows(doc.frontmatter).map((r) => ({ label: r.key, value: valueNode(r.value), minMode: r.advanced ? "advanced" : undefined }))
    : [];
  const fallbackTitle = path === "" ? "Knowledge base" : (path.split("/").pop() ?? path).replace(/\.md$/i, "");

  return (
    <div className="space-y-6">
      <SectionHeader level={1} kicker="knowledge base" title={doc.title ?? fallbackTitle} />
      {path !== "" ? (
        <p className={TEXT}>
          <RouteLink to="/kb">All entries</RouteLink> <span aria-hidden="true">/</span> <span className="break-all">{path}</span>
        </p>
      ) : null}
      {rows.length > 0 ? <SpecTable mode={mode} rows={rows} /> : null}
      <article className="max-w-3xl">
        <Blocks blocks={doc.blocks} />
      </article>
      {doc.truncated ? <p className={TEXT}>This entry is very long. Only the first part is shown.</p> : null}
    </div>
  );
}

export default function KbView() {
  const [routePath] = useAppLocation();
  const target = kbRouteTarget(routePath);
  const { load, retry } = useKbText(target);
  const browse = (
    <ButtonLink href={hrefTo("/kb")} size="sm">
      Browse the knowledge base
    </ButtonLink>
  );

  if (target.kind === "invalid") {
    return (
      <Empty kicker="400" title="That is not a valid knowledge-base path" action={browse}>
        Entries are relative file paths such as projects/meshtastic.md.
      </Empty>
    );
  }
  if (load.state === "loading") {
    return (
      <p role="status" className={TEXT}>
        Reading the knowledge base{"…"}
      </p>
    );
  }
  if (load.state === "error") {
    if (load.kind === "not_found") {
      return (
        <Empty kicker="404" title="No such entry" action={browse}>
          {target.kind === "entry" ? `The knowledge base on this node has nothing at ${target.path}.` : "This node has no knowledge-base index."}
        </Empty>
      );
    }
    if (load.kind === "bad_request") {
      return (
        <Empty kicker="400" title="The node refused that path" action={browse}>
          {load.message}
        </Empty>
      );
    }
    return (
      <Empty
        kicker="unavailable"
        title="Could not load this page"
        action={
          <Button size="sm" onClick={retry}>
            Try again
          </Button>
        }
      >
        {load.message}
      </Empty>
    );
  }
  return <Document text={load.text} path={target.kind === "entry" ? target.path : ""} />;
}
