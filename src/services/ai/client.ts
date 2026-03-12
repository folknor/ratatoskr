import { invoke } from "@tauri-apps/api/core";
import { AiError, type AiErrorCode } from "./errors";
import type { AiCompletionRequest } from "./types";

function toAiError(error: unknown): AiError {
  if (error instanceof AiError) return error;

  const message = error instanceof Error ? error.message : String(error);
  const match = message.match(
    /^(NOT_CONFIGURED|AUTH_ERROR|RATE_LIMITED|NETWORK_ERROR):\s*(.*)$/s,
  );

  if (match) {
    const code = match[1];
    const detail = match[2];
    if (code) {
      return new AiError(code as AiErrorCode, detail || code);
    }
  }

  return new AiError("NETWORK_ERROR", message);
}

export async function completeAi(req: AiCompletionRequest): Promise<string> {
  try {
    return await invoke<string>("ai_complete", { request: req });
  } catch (error) {
    throw toAiError(error);
  }
}

export async function callAi(
  systemPrompt: string,
  userContent: string,
): Promise<string> {
  return completeAi({ systemPrompt, userContent });
}

export async function testAiConnection(): Promise<boolean> {
  try {
    return await invoke<boolean>("ai_test_connection");
  } catch (error) {
    throw toAiError(error);
  }
}
