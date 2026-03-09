import { GoogleGenerativeAI } from "@google/generative-ai";
import { createProviderFactory } from "../providerFactory";
import type { AiCompletionRequest, AiProviderClient } from "../types";

const factory: ReturnType<typeof createProviderFactory<GoogleGenerativeAI>> =
  createProviderFactory((apiKey) => new GoogleGenerativeAI(apiKey));

export function createGeminiProvider(
  apiKey: string,
  modelId: string,
): AiProviderClient {
  const client = factory.getClient(apiKey);

  return {
    async complete(req: AiCompletionRequest): Promise<string> {
      const model = client.getGenerativeModel({
        model: modelId,
        systemInstruction: req.systemPrompt,
      });

      const result = await model.generateContent(req.userContent);
      return result.response.text();
    },

    async testConnection(): Promise<boolean> {
      try {
        const model = client.getGenerativeModel({
          model: modelId,
        });
        await model.generateContent("Say hi");
        return true;
      } catch {
        return false;
      }
    },
  };
}

export function clearGeminiProvider(): void {
  factory.clear();
}
