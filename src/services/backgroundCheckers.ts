/**
 * Factory for creating background interval checkers.
 * Provides consistent start/stop/error handling for periodic tasks.
 */
export interface BackgroundChecker {
  start(): void;
  stop(): void;
}

export function createBackgroundChecker(
  name: string,
  checkFn: () => Promise<void>,
  intervalMs: number = 60_000,
): BackgroundChecker {
  let interval: ReturnType<typeof setInterval> | null = null;
  let running = false;

  const run = async (): Promise<void> => {
    if (running) return; // Prevent overlapping runs
    running = true;
    try {
      await checkFn();
    } catch (err) {
      console.error(`[${name}] check failed:`, err);
    } finally {
      running = false;
    }
  };

  return {
    start(): void {
      if (interval) return;
      void run();
      interval = setInterval(run, intervalMs);
    },
    stop(): void {
      if (interval) {
        clearInterval(interval);
        interval = null;
      }
    },
  };
}
