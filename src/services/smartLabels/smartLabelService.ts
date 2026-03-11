import { classifyThreadsBySmartLabels } from "@/services/ai/aiService";
import type { FilterCriteria } from "@/services/db/filters";
import { getEnabledSmartLabelRules } from "@/services/db/smartLabelRules";
import { messageMatchesFilter } from "@/services/filters/filterEngine";
import type { ParsedMessage } from "@/services/gmail/messageParser";

export interface SmartLabelMatch {
  threadId: string;
  labelIds: string[];
}

export interface SmartLabelAIThread {
  id: string;
  subject: string;
  snippet: string;
  fromAddress: string;
}

export interface SmartLabelAIRule {
  labelId: string;
  description: string;
}

function toPairKey(threadId: string, labelId: string): string {
  return `${threadId}:${labelId}`;
}

/**
 * Match messages against smart label rules using two-phase matching:
 * 1. Fast path: traditional filter criteria (deterministic)
 * 2. AI path: batch remaining unmatched threads to AI
 */
export async function matchSmartLabels(
  accountId: string,
  messages: ParsedMessage[],
  preAppliedMatches: SmartLabelMatch[] = [],
): Promise<SmartLabelMatch[]> {
  const rules = await getEnabledSmartLabelRules(accountId);
  if (rules.length === 0) return [];

  // Deduplicate threads — use first message per thread for matching
  const threadMap = new Map<string, ParsedMessage>();
  for (const msg of messages) {
    if (!threadMap.has(msg.threadId)) {
      threadMap.set(msg.threadId, msg);
    }
  }

  // Phase 1: Fast path — check criteria for rules that have them
  const knownMatches = new Map<string, Set<string>>(); // threadId → labelIds
  const matchesToApply = new Map<string, Set<string>>(); // threadId → labelIds
  const rulesWithCriteria: { labelId: string; criteria: FilterCriteria }[] = [];
  const allRulesForAi: { labelId: string; description: string }[] = [];

  const preAppliedPairs = new Set<string>();
  for (const match of preAppliedMatches) {
    const existing = knownMatches.get(match.threadId) ?? new Set<string>();
    for (const labelId of match.labelIds) {
      existing.add(labelId);
      preAppliedPairs.add(toPairKey(match.threadId, labelId));
    }
    knownMatches.set(match.threadId, existing);
  }

  for (const rule of rules) {
    allRulesForAi.push({
      labelId: rule.label_id,
      description: rule.ai_description,
    });

    if (rule.criteria_json) {
      try {
        const criteria = JSON.parse(rule.criteria_json) as FilterCriteria;
        if (Object.keys(criteria).length > 0) {
          rulesWithCriteria.push({ labelId: rule.label_id, criteria });
        }
      } catch {
        // Invalid criteria JSON, skip fast path for this rule
      }
    }
  }

  // Track which threadId+labelId combos were matched by criteria
  const knownMatchedPairs = new Set<string>(preAppliedPairs);

  for (const [threadId, msg] of threadMap) {
    for (const { labelId, criteria } of rulesWithCriteria) {
      if (preAppliedPairs.has(toPairKey(threadId, labelId))) continue;
      if (messageMatchesFilter(msg, criteria)) {
        const existing = knownMatches.get(threadId) ?? new Set();
        existing.add(labelId);
        knownMatches.set(threadId, existing);

        const applyExisting = matchesToApply.get(threadId) ?? new Set();
        applyExisting.add(labelId);
        matchesToApply.set(threadId, applyExisting);

        knownMatchedPairs.add(toPairKey(threadId, labelId));
      }
    }
  }

  // Phase 2: AI path — classify threads that weren't fully matched by criteria
  // Send all threads to AI for labels that didn't match via criteria
  const threadsForAi: {
    id: string;
    subject: string;
    snippet: string;
    fromAddress: string;
  }[] = [];
  for (const [threadId, msg] of threadMap) {
    // Include thread if any label rule hasn't been matched by criteria for this thread
    const matchedLabels = knownMatches.get(threadId);
    const allLabelsMatched = allRulesForAi.every((r) =>
      matchedLabels?.has(r.labelId),
    );
    if (!allLabelsMatched) {
      threadsForAi.push({
        id: threadId,
        subject: msg.subject ?? "",
        snippet: msg.snippet,
        fromAddress: msg.fromAddress ?? "",
      });
    }
  }

  if (threadsForAi.length > 0 && allRulesForAi.length > 0) {
    try {
      const aiResults = await classifyThreadsBySmartLabels(
        threadsForAi,
        allRulesForAi,
      );

      // Merge AI results (skip pairs already matched by criteria)
      for (const [threadId, labelIds] of aiResults) {
        const existing = knownMatches.get(threadId) ?? new Set();
        const applyExisting = matchesToApply.get(threadId) ?? new Set();
        for (const labelId of labelIds) {
          if (!knownMatchedPairs.has(toPairKey(threadId, labelId))) {
            existing.add(labelId);
            applyExisting.add(labelId);
            knownMatchedPairs.add(toPairKey(threadId, labelId));
          }
        }
        if (existing.size > 0) {
          knownMatches.set(threadId, existing);
        }
        if (applyExisting.size > 0) {
          matchesToApply.set(threadId, applyExisting);
        }
      }
    } catch (err) {
      console.error("Smart label AI classification failed:", err);
      // Continue with criteria-only matches
    }
  }

  // Convert to result array
  const results: SmartLabelMatch[] = [];
  for (const [threadId, labelIds] of matchesToApply) {
    results.push({ threadId, labelIds: [...labelIds] });
  }

  return results;
}

export async function classifySmartLabelRemainder(
  threads: SmartLabelAIThread[],
  rules: SmartLabelAIRule[],
  preAppliedMatches: SmartLabelMatch[] = [],
): Promise<SmartLabelMatch[]> {
  if (threads.length === 0 || rules.length === 0) {
    return [];
  }

  const preAppliedPairs = new Set<string>();
  for (const match of preAppliedMatches) {
    for (const labelId of match.labelIds) {
      preAppliedPairs.add(toPairKey(match.threadId, labelId));
    }
  }

  const aiResults = await classifyThreadsBySmartLabels(threads, rules);
  const results: SmartLabelMatch[] = [];
  for (const [threadId, labelIds] of aiResults) {
    const unappliedLabelIds = labelIds.filter(
      (labelId) => !preAppliedPairs.has(toPairKey(threadId, labelId)),
    );
    if (unappliedLabelIds.length > 0) {
      results.push({ threadId, labelIds: unappliedLabelIds });
    }
  }
  return results;
}
