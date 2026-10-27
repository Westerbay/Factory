/** Compare normalized JSON event content; object member order is not persisted meaning. */
export function sameEventContent(left: unknown, right: unknown): boolean {
  if (left === right) return true
  if (Array.isArray(left)) {
    if (!Array.isArray(right) || left.length !== right.length) return false
    return left.every((value: unknown, index: number) =>
      sameEventContent(value, right[index])
    )
  }
  if (
    typeof left !== "object" ||
    left === null ||
    typeof right !== "object" ||
    right === null ||
    Array.isArray(right)
  )
    return false
  // Optional undefined members disappear during JSON storage; compare their stored meaning.
  const leftEntries = Object.entries(left).filter(
    ([, value]: [string, unknown]) => value !== undefined
  )
  const rightEntries = new Map<string, unknown>(
    Object.entries(right).filter(
      ([, value]: [string, unknown]) => value !== undefined
    )
  )
  if (leftEntries.length !== rightEntries.size) return false
  return leftEntries.every(
    ([key, value]: [string, unknown]) =>
      rightEntries.has(key) && sameEventContent(value, rightEntries.get(key))
  )
}
