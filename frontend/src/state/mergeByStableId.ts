export function mergeByStableId<T extends { id: string }>(current: T[], next: T[]): T[] {
  const values = new Map(current.map(value => [value.id, value]))
  next.forEach(value => values.set(value.id, value))
  return Array.from(values.values())
}
