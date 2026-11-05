/**
 * Arrow keys through the run list (2.10d.3): which row a key moves to. Pure, so `node --test` covers it.
 * `current` is the focused row's index, or -1 when focus is in the filter box above the list.
 */
export function nextIndex(key: string, current: number, count: number): number | null {
  if (count <= 0) return null;
  if (current < 0) return key === "ArrowDown" ? 0 : null;
  let next: number;
  switch (key) {
    case "ArrowDown":
      next = Math.min(current + 1, count - 1);
      break;
    case "ArrowUp":
      next = Math.max(current - 1, 0);
      break;
    case "Home":
      next = 0;
      break;
    case "End":
      next = count - 1;
      break;
    default:
      return null;
  }
  return next === current ? null : next;
}
