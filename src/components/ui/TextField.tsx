import { type InputHTMLAttributes, forwardRef } from "react";

interface TextFieldProps extends Omit<InputHTMLAttributes<HTMLInputElement>, "size"> {
  label?: string;
  size?: "sm" | "md";
  error?: string;
}

export const TextField = forwardRef<HTMLInputElement, TextFieldProps>(function TextField(
  { label, size = "sm", error, className = "", id, ...rest },
  ref,
) {
  const inputId = id ?? (label ? label.toLowerCase().replace(/\s+/g, "-") : undefined);

  const sizes = {
    sm: "px-3 py-1.5 text-sm",
    md: "px-3 py-2 text-sm",
  };

  return (
    <div className={className}>
      {label && (
        <label htmlFor={inputId} className="text-sm text-text-secondary block mb-1.5">
          {label}
        </label>
      )}
      <input
        ref={ref}
        id={inputId}
        className={`w-full ${sizes[size]} bg-bg-tertiary border ${error ? "border-danger" : "border-border-primary"} rounded text-text-primary outline-none focus:border-accent`}
        {...rest}
      />
      {error && <p className="text-xs text-danger mt-1">{error}</p>}
    </div>
  );
});
