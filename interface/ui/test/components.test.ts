// Server-rendered checks of the small shared components: what they put in the HTML is what a keyboard
// or screen-reader user gets (no DOM needed).
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { Button } from "../src/components/button";
import { Panel } from "../src/components/panel";
import { Toggle } from "../src/components/toggle";
import { parseMarkdown } from "../src/lib/markdown";
import { Blocks, minHeadingLevel } from "../src/views/kb/render";

const html = (el: Parameters<typeof renderToStaticMarkup>[0]) => renderToStaticMarkup(el);

describe("Button keeps focus while it works", () => {
  it("loading is aria-disabled, not disabled (a disabled button drops focus to the page body)", () => {
    const out = html(createElement(Button, { loading: true }, "Check now"));
    expect(out).toContain('aria-disabled="true"');
    expect(out).toContain('aria-busy="true"');
    expect(out).not.toMatch(/\sdisabled(=|\s|>)/);
  });
  it("a plain disabled button is still disabled", () => {
    expect(html(createElement(Button, { disabled: true }, "Go"))).toMatch(/\sdisabled(=|\s|>)/);
  });
  it("aria-disabled passed in is honoured without the disabled attribute", () => {
    const out = html(createElement(Button, { "aria-disabled": true }, "send"));
    expect(out).toContain('aria-disabled="true"');
    expect(out).not.toMatch(/\sdisabled(=|\s|>)/);
  });
  it("an idle button carries neither", () => {
    const out = html(createElement(Button, null, "Go"));
    expect(out).not.toContain('aria-disabled="');
    expect(out).not.toContain('aria-busy="');
  });
});

describe("Toggle", () => {
  it("busy keeps it focusable but marks it disabled", () => {
    const out = html(createElement(Toggle, { pressed: true, onPressedChange: () => undefined, busy: true, children: "LAN sharing" }));
    expect(out).toContain('aria-disabled="true"');
    expect(out).toContain('aria-pressed="true"');
    expect(out).not.toMatch(/\sdisabled(=|\s|>)/);
  });
});

describe("Panel heading", () => {
  it("exposes the title as a visually hidden heading when asked, nothing otherwise", () => {
    expect(html(createElement(Panel, { title: "LoRa board", heading: 2 }, "x"))).toContain('<h2 class="sr-only">LoRa board</h2>');
    expect(html(createElement(Panel, { title: "LoRa board" }, "x"))).not.toContain("<h2");
  });
});

describe("markdown headings", () => {
  const md = "## Install\n\ntext\n\n### Flash\n\nmore\n\n## Check\n";
  const doc = parseMarkdown(md);
  it("an entry that starts at ## gets an h2 under the page's h1, never an h3 first", () => {
    expect(minHeadingLevel(doc.blocks)).toBe(2);
    const out = html(createElement(Blocks, { blocks: doc.blocks, minLevel: minHeadingLevel(doc.blocks) }));
    expect(out).toMatch(/<h2[^>]*>Install/);
    expect(out).toMatch(/<h3[^>]*>Flash/);
    expect(out).not.toContain("<h1");
  });
  it("without a minimum, levels keep their old offset", () => {
    expect(html(createElement(Blocks, { blocks: doc.blocks }))).toMatch(/<h3[^>]*>Install/);
  });
  it("a body inside a card shows headings as bold lines and adds no headings to the page", () => {
    const out = html(createElement(Blocks, { blocks: doc.blocks, headingsAsBold: true, compact: true }));
    expect(out).not.toMatch(/<h[1-6]/);
    expect(out).toMatch(/<p class="mt-2 font-bold[^"]*">Install/);
  });
  it("no headings at all is level 1", () => {
    expect(minHeadingLevel(parseMarkdown("just text").blocks)).toBe(1);
  });
});
