import type React from "react";
import { useCallback, useEffect, useRef, useState } from "react";
import { Button } from "./Button";
import { Modal } from "./Modal";

interface InputField {
  key: string;
  label: string;
  placeholder?: string;
  defaultValue?: string;
  required?: boolean;
}

interface InputDialogProps {
  isOpen: boolean;
  onClose: () => void;
  onSubmit: (values: Record<string, string>) => void;
  title: string;
  fields: InputField[];
  submitLabel?: string;
}

export function InputDialog({
  isOpen,
  onClose,
  onSubmit,
  title,
  fields,
  submitLabel = "Save",
}: InputDialogProps): React.ReactNode {
  const buildInitial = useCallback(
    () => Object.fromEntries(fields.map((f) => [f.key, f.defaultValue ?? ""])),
    [fields],
  );

  const [values, setValues] = useState<Record<string, string>>(buildInitial);
  const firstInputRef = useRef<HTMLInputElement>(null);

  // Reset values when the dialog opens or fields change
  useEffect((): (() => void) | undefined => {
    if (isOpen) {
      setValues(buildInitial());
      const id = setTimeout(() => firstInputRef.current?.focus(), 50);
      return () => clearTimeout(id);
    }
    return;
  }, [isOpen, buildInitial]);

  const isValid = fields.every((f) => {
    const required = f.required ?? true;
    return !required || values[f.key]?.trim();
  });

  const handleSubmit = (): void => {
    if (!isValid) return;
    onSubmit(values);
    onClose();
  };

  const handleKeyDown = (e: React.KeyboardEvent): void => {
    if (e.key === "Enter" && fields.length === 1 && isValid) {
      e.preventDefault();
      handleSubmit();
    }
  };

  return (
    <Modal isOpen={isOpen} onClose={onClose} title={title} width="w-96">
      {/* biome-ignore lint/a11y/noStaticElementInteractions: keyDown handler for form submission */}
      <div className="p-4 space-y-3" onKeyDown={handleKeyDown}>
        {fields.map((field, i) => (
          <div key={field.key}>
            <label
              htmlFor={`input-dialog-${field.key}`}
              className="block text-xs font-medium text-text-secondary mb-1"
            >
              {field.label}
            </label>
            <input
              id={`input-dialog-${field.key}`}
              ref={i === 0 ? firstInputRef : undefined}
              type="text"
              value={values[field.key] ?? ""}
              // biome-ignore lint/nursery/useExplicitType: inline callback
              onChange={(e) =>
                setValues((prev) => ({ ...prev, [field.key]: e.target.value }))
              }
              placeholder={field.placeholder}
              className="w-full bg-bg-tertiary text-text-primary text-sm px-3 py-1.5 rounded-md border border-border-primary focus:border-accent focus:outline-none placeholder:text-text-tertiary"
            />
          </div>
        ))}
        <div className="flex justify-end gap-2 pt-1">
          <Button variant="secondary" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" onClick={handleSubmit} disabled={!isValid}>
            {submitLabel}
          </Button>
        </div>
      </div>
    </Modal>
  );
}
