/**
 * Core facade for label operations.
 *
 * UI code (components, hooks, stores) should import from here
 * instead of reaching into @/services/db/labels directly.
 */

export {
  type DbLabel,
  deleteLabel,
  getLabelsForAccount,
  updateLabelSortOrder,
  upsertLabel,
} from "@/services/db/labels";
