# Planned — placeholder wikilink registry

A prose `[[wikilink]]` that resolves to no existing entry is a lint warning UNLESS its slug is registered here (schema B07). This file is the only legal home for intentional dangling links.

Rules:

- One slug per line, with a one-line reason the entry will exist.
- Register the slug before, or in the same commit as, the prose link that uses it.
- Remove the line in the same commit that adds the real entry.
- Relations-graph targets may NEVER be registered here — edges must point at existing entries (schema R02).
- If a registered slug later collides with a real entry through case/kebab normalization, the link is a did-you-mean error (schema B08): fix the link and drop the line.

## Registry

(the list is empty — format: `- slug — reason`)

<!-- example:
- identity-systems-inventory — deferred Layer 2 identity survey; the Layer 2 research session drafts the entries
-->
