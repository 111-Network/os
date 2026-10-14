// Licenses (`/settings/licenses`): this node's own license and the third-party notices, both shipped inside the
// dashboard (imported at build time with `?raw`, so the daemon serves them from the same embedded dist) and drawn
// by the safe markdown renderer the knowledge base uses. The notices file is long: it is loaded as a chunk of its
// own, and its text is only parsed and drawn once the reader opens the disclosure.
import { useEffect, useMemo, useState } from "react";
import { hrefTo } from "../../app/location";
import { Button, ButtonLink } from "../../components/button";
import { Disclosure } from "../../components/disclosure";
import { Panel } from "../../components/panel";
import { SectionHeader } from "../../components/section-header";
import { parseMarkdown } from "../../lib/markdown";
import { Blocks, minHeadingLevel } from "../kb/render";
import { NOTICES_FAILED, NOTICES_LOADING, countLines, describeDoc, noticesLabel } from "./model";
import license from "../../../../../LICENSE.md?raw";

const TEXT = "font-mono text-xs leading-relaxed text-muted-foreground";

/** One markdown document. Headings start one level below the panel's own (sr-only) h2. */
function Doc({ text }: { text: string }) {
  const doc = useMemo(() => parseMarkdown(text), [text]);
  return (
    <article className="max-w-3xl break-words">
      <Blocks blocks={doc.blocks} minLevel={minHeadingLevel(doc.blocks) - 1} />
      {doc.truncated ? <p className={`mt-3 ${TEXT}`}>This document is very long. Only the first part is shown.</p> : null}
    </article>
  );
}

type Notices = { state: "loading" } | { state: "ok"; text: string } | { state: "error" };

/** The notices chunk: a dynamic import, so it is a file of its own in dist and not part of this view's chunk. */
function useNotices(): { notices: Notices; retry: () => void } {
  const [notices, setNotices] = useState<Notices>({ state: "loading" });
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    let alive = true;
    setNotices({ state: "loading" });
    import("../../../../../THIRD-PARTY-NOTICES.md?raw").then(
      (m) => alive && setNotices({ state: "ok", text: m.default }),
      () => alive && setNotices({ state: "error" }),
    );
    return () => {
      alive = false;
    };
  }, [attempt]);
  return { notices, retry: () => setAttempt((n) => n + 1) };
}

function NoticesPanel() {
  const { notices, retry } = useNotices();
  const [open, setOpen] = useState(false);
  const lines = notices.state === "ok" ? countLines(notices.text) : 0;
  return (
    <Panel title="third-party notices" heading={2} chromeEnd={notices.state === "ok" ? describeDoc(notices.text) : undefined} bodyClassName="flex flex-col gap-3">
      <p className={`max-w-prose ${TEXT}`}>
        The licenses of the components this node includes, reproduced so their notice and attribution requirements are met.
      </p>
      {notices.state === "ok" ? (
        <Disclosure summary={noticesLabel(lines)} onToggle={(e) => setOpen(e.currentTarget.open)}>
          {open ? <Doc text={notices.text} /> : null}
        </Disclosure>
      ) : notices.state === "loading" ? (
        <p role="status" className={TEXT}>
          {NOTICES_LOADING}
        </p>
      ) : (
        <div className="flex flex-col items-start gap-2">
          <p role="alert" className="font-mono text-xs leading-relaxed text-destructive">
            {NOTICES_FAILED}
          </p>
          <Button size="sm" onClick={retry}>
            Try again
          </Button>
        </div>
      )}
    </Panel>
  );
}

export default function LicensesView() {
  return (
    <div className="flex flex-col gap-6">
      <SectionHeader
        level={1}
        kicker="settings"
        title="Licenses"
        end={
          <ButtonLink href={hrefTo("/settings")} size="sm">
            Settings
          </ButtonLink>
        }
      />
      <p className={`max-w-prose ${TEXT}`}>
        The license this node is distributed under, and the licenses of the third-party components it includes. Both are part of this dashboard; nothing is
        downloaded to show them.
      </p>
      <Panel title="license" heading={2} chromeEnd={describeDoc(license)}>
        <Doc text={license} />
      </Panel>
      <NoticesPanel />
    </div>
  );
}
