// Table lookups for values that come from the node (a state, a reason code, a role). A plain-object table answers
// "constructor" and "toString" for those keys, so every lookup in the Messaging UI goes through this guard and an
// unknown value falls back to "unknown" (the U1b rule) instead of a function from Object.prototype.

/** True when `key` is an own property of `table`. Narrows `key` to the table's keys. */
export function own<T extends object>(table: T, key: string): key is Extract<keyof T, string> {
  return Object.prototype.hasOwnProperty.call(table, key);
}
