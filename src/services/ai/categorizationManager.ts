import { invoke } from "@tauri-apps/api/core";
import { categorizeThreads } from "./aiService";
import { isAiAvailable } from "./providerManager";

export interface CategorizationCandidate {
  id: string;
  subject?: string | null;
  snippet?: string | null;
  fromAddress?: string | null;
}

export async function categorizeNewThreads(
  accountId: string,
  candidates: CategorizationCandidate[],
): Promise<void> {
  try {
    const aiAvail = await isAiAvailable();
    if (!aiAvail) return;
    if (candidates.length === 0) return;

    // Categorize via AI (refines rule-based results)
    const categories = await categorizeThreads(
      candidates.map((t) => ({
        id: t.id,
        subject: t.subject ?? "",
        snippet: t.snippet ?? "",
        fromAddress: t.fromAddress ?? "",
      })),
    );

    if (categories.size === 0) return;

    await invoke("categorization_apply_ai_results", {
      accountId,
      categories: Array.from(categories.entries()),
    });
  } catch (err) {
    // Non-blocking — log and continue
    console.error("Auto-categorization failed:", err);
  }
}
