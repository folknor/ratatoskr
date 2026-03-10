import type React from "react";

export interface StepDef {
  key: string;
  label: string;
  icon: React.ReactNode;
}

interface StepIndicatorProps {
  steps: StepDef[];
  currentStepIndex: number;
}

export function StepIndicator({
  steps,
  currentStepIndex,
}: StepIndicatorProps): React.ReactNode {
  return (
    <div className="flex items-center justify-center gap-1 mb-6">
      {steps.map((step, i) => {
        const isActive = i === currentStepIndex;
        const isCompleted = i < currentStepIndex;
        return (
          <div key={step.key} className="flex items-center gap-1">
            {i > 0 && (
              <div
                className={`w-6 h-px ${isCompleted ? "bg-accent" : "bg-border-primary"}`}
              />
            )}
            <div
              className={`flex items-center gap-1.5 px-2 py-1 rounded-md text-xs font-medium transition-colors ${
                isActive
                  ? "bg-accent/10 text-accent"
                  : isCompleted
                    ? "text-accent"
                    : "text-text-tertiary"
              }`}
            >
              {step.icon}
              <span className="hidden sm:inline">{step.label}</span>
            </div>
          </div>
        );
      })}
    </div>
  );
}
