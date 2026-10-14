import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { parseMarkdown } from "../src/lib/markdown";
import { minModeFor, titleFor } from "../src/lib/nav";
import { NOTICES_FAILED, countLines, describeDoc, firstHeading, linesText, noticesLabel } from "../src/views/licenses/model";

describe("countLines", () => {
  it("counts lines as an editor does", () => {
    expect(countLines("")).toBe(0);
    expect(countLines("a")).toBe(1);
    expect(countLines("a\n")).toBe(1);
    expect(countLines("a\nb")).toBe(2);
    expect(countLines("a\n\nb\n")).toBe(3);
    expect(countLines("a\r\nb\r\n")).toBe(2);
    expect(countLines("\n")).toBe(1);
  });
  it("names the count in words, singular for one", () => {
    expect(linesText(1)).toBe("1 line");
    expect(linesText(0)).toBe("0 lines");
    expect(noticesLabel(1071)).toBe("Show all notices (1071 lines)");
  });
});

describe("describeDoc", () => {
  it("names the document and its size, or only the size when it has no title", () => {
    expect(describeDoc("# 111network License\n\ntext\n")).toBe("111network License · 3 lines");
    expect(describeDoc("no title\nat all")).toBe("2 lines");
    expect(describeDoc("")).toBe("0 lines");
  });
});

describe("firstHeading", () => {
  it("takes the first level-1 heading", () => {
    expect(firstHeading("# Third-party notices\n\ntext\n\n# Later")).toBe("Third-party notices");
    expect(firstHeading("intro\n\n#   Spaced out   \n")).toBe("Spaced out");
    expect(firstHeading("# Closed #\n")).toBe("Closed");
  });
  it("skips deeper headings, fenced code and text that only looks like a heading", () => {
    expect(firstHeading("## Not this\n\n#hashtag\n\n# This one")).toBe("This one");
    expect(firstHeading("```\n# in a code block\n```\n# After")).toBe("After");
    expect(firstHeading("~~~sh\n# comment\n~~~\n\n# Real")).toBe("Real");
    expect(firstHeading("```\n# never closed\n")).toBeNull();
    expect(firstHeading("no heading here")).toBeNull();
    expect(firstHeading("")).toBeNull();
    expect(firstHeading("#\n")).toBeNull();
  });
});

describe("the two documents the page ships", () => {
  // The view imports these with `?raw` from five directories up; this test reads the same files from the same place.
  const read = (name: string) => readFileSync(new URL(`../../../${name}`, import.meta.url), "utf8");
  const license = read("LICENSE.md");
  const notices = read("THIRD-PARTY-NOTICES.md");

  it("exist where the view looks for them and have a title", () => {
    expect(firstHeading(license)).toBeTruthy();
    expect(firstHeading(notices)).toBe("Third-party notices");
    expect(countLines(license)).toBeGreaterThan(20);
    expect(countLines(notices)).toBeGreaterThan(500);
  });
  it("parse whole, with the title dropped because the page names the document", () => {
    for (const text of [license, notices]) {
      const doc = parseMarkdown(text);
      expect(doc.truncated).toBe(false);
      expect(doc.blocks.length).toBeGreaterThan(10);
      expect(doc.blocks[0]?.t).not.toBe("heading"); // the leading `# Title` is not repeated in the body
    }
  });
});

describe("route and title", () => {
  it("is called Licenses, and is for everyone", () => {
    expect(titleFor("/settings/licenses")).toBe("Licenses");
    expect(titleFor("/settings/l3")).toBe("Intelligence · L3"); // layers keep their own titles
    expect(minModeFor("/settings/licenses")).toBeUndefined();
  });
  it("has an honest failure sentence", () => {
    expect(NOTICES_FAILED).toMatch(/could not be loaded/);
  });
});
