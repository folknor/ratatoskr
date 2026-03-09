import { vi } from "vitest";

export function createMockDb(): {
  select: ReturnType<typeof vi.fn>;
  execute: ReturnType<typeof vi.fn>;
} {
  return {
    select: vi.fn(() => Promise.resolve([])),
    execute: vi.fn(() => Promise.resolve({ rowsAffected: 1 })),
  };
}
