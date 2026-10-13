export type ClassValue = string | false | null | undefined;

/** Join class names, dropping falsy values. */
export const cx = (...parts: ClassValue[]): string => parts.filter(Boolean).join(" ");

/** Shared focus treatment for every interactive element (DESIGN.md section 8). */
export const FOCUS_RING = "focus-visible:ring-2 focus-visible:ring-foreground focus-visible:outline-none";
