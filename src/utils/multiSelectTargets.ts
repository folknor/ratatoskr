/**
 * Resolves which thread IDs to act on, accounting for multi-selection state.
 *
 * Two usage patterns:
 *
 * 1. **Context menu** (`resolveContextMenuTargets`): A specific thread was right-clicked.
 *    If that thread is part of a multi-selection, act on all selected threads;
 *    otherwise act on just the right-clicked thread.
 *
 * 2. **Keyboard shortcut** (`resolveKeyboardTargets`): No specific thread was clicked.
 *    If there's a multi-selection, act on all selected threads;
 *    otherwise act on the currently focused/selected thread (if any).
 */

/**
 * Resolve target thread IDs for a context menu action.
 *
 * @param threadId - The thread that was right-clicked
 * @param selectedThreadIds - The current multi-selection set
 * @returns An array of thread IDs to act on, and whether it's a multi-action
 */
export function resolveContextMenuTargets(
  threadId: string,
  selectedThreadIds: ReadonlySet<string>,
): { targetIds: string[]; isMulti: boolean } {
  const isInMultiSelect = selectedThreadIds.has(threadId);
  const targetIds =
    isInMultiSelect && selectedThreadIds.size > 1
      ? [...selectedThreadIds]
      : [threadId];
  return { targetIds, isMulti: targetIds.length > 1 };
}

/**
 * Resolve target thread IDs for a keyboard shortcut action.
 *
 * @param selectedThreadIds - The current multi-selection set
 * @param focusedThreadId - The currently focused/selected single thread (if any)
 * @returns An array of thread IDs to act on
 */
export function resolveKeyboardTargets(
  selectedThreadIds: ReadonlySet<string>,
  focusedThreadId: string | null | undefined,
): string[] {
  if (selectedThreadIds.size > 0) {
    return [...selectedThreadIds];
  }
  if (focusedThreadId) {
    return [focusedThreadId];
  }
  return [];
}
