import { Users } from "lucide-react";
import type React from "react";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  type DbContact,
  type DbContactGroup,
  expandContactGroup,
  searchContactGroups,
  searchContacts,
} from "@/core/queries";

type AutocompleteSuggestion =
  | { kind: "contact"; data: DbContact }
  | { kind: "group"; data: DbContactGroup };

interface AddressInputProps {
  label: string;
  addresses: string[];
  onChange: (addresses: string[]) => void;
  placeholder?: string;
  autoFocus?: boolean;
}

export function AddressInput({
  label,
  addresses,
  onChange,
  placeholder,
  autoFocus = false,
}: AddressInputProps): React.ReactNode {
  const { t } = useTranslation("composer");
  const resolvedPlaceholder = placeholder ?? t("addRecipients");
  const [inputValue, setInputValue] = useState("");
  const [suggestions, setSuggestions] = useState<AutocompleteSuggestion[]>([]);
  const [showSuggestions, setShowSuggestions] = useState(false);
  const [selectedIdx, setSelectedIdx] = useState(-1);
  const inputRef = useRef<HTMLInputElement>(null);
  const blurTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const searchTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(
    // biome-ignore lint/nursery/useExplicitType: cleanup effect
    () => () => {
      if (blurTimerRef.current) clearTimeout(blurTimerRef.current);
      if (searchTimerRef.current) clearTimeout(searchTimerRef.current);
    },
    [],
  );

  useEffect(() => {
    if (autoFocus) {
      inputRef.current?.focus();
    }
  }, [autoFocus]);

  const handleInputChange = useCallback((value: string): void => {
    setInputValue(value);
    if (searchTimerRef.current) clearTimeout(searchTimerRef.current);
    if (value.length >= 2) {
      searchTimerRef.current = setTimeout(async () => {
        const [contacts, groups] = await Promise.all([
          searchContacts(value, 5),
          searchContactGroups(value, 3),
        ]);
        const merged: AutocompleteSuggestion[] = [
          ...groups.map(
            (g): AutocompleteSuggestion => ({ kind: "group", data: g }),
          ),
          ...contacts.map(
            (c): AutocompleteSuggestion => ({ kind: "contact", data: c }),
          ),
        ];
        setSuggestions(merged);
        setShowSuggestions(merged.length > 0);
        setSelectedIdx(-1);
      }, 200);
    } else {
      setSuggestions([]);
      setShowSuggestions(false);
    }
  }, []);

  const addAddress = useCallback(
    (address: string) => {
      const trimmed = address.trim();
      if (trimmed && !addresses.includes(trimmed)) {
        onChange([...addresses, trimmed]);
      }
      setInputValue("");
      setSuggestions([]);
      setShowSuggestions(false);
      inputRef.current?.focus();
    },
    [addresses, onChange],
  );

  const addAddresses = useCallback(
    (newAddresses: string[]) => {
      const unique = newAddresses.filter(
        (a) => a.trim() && !addresses.includes(a),
      );
      if (unique.length > 0) {
        onChange([...addresses, ...unique]);
      }
      setInputValue("");
      setSuggestions([]);
      setShowSuggestions(false);
      inputRef.current?.focus();
    },
    [addresses, onChange],
  );

  const selectSuggestion = useCallback(
    async (suggestion: AutocompleteSuggestion): Promise<void> => {
      if (suggestion.kind === "contact") {
        addAddress(suggestion.data.email);
      } else {
        const emails = await expandContactGroup(suggestion.data.id);
        addAddresses(emails);
      }
    },
    [addAddress, addAddresses],
  );

  const removeAddress = useCallback(
    (index: number) => {
      onChange(addresses.filter((_, i) => i !== index));
    },
    [addresses, onChange],
  );

  const handleKeyDown = (e: React.KeyboardEvent): void => {
    if (e.key === "Enter" || e.key === "Tab" || e.key === ",") {
      if (showSuggestions && selectedIdx >= 0) {
        e.preventDefault();
        const selected = suggestions[selectedIdx];
        if (selected) {
          void selectSuggestion(selected);
        }
      } else if (inputValue.trim()) {
        e.preventDefault();
        addAddress(inputValue);
      } else if (e.key !== "Tab") {
        e.preventDefault();
      }
    } else if (e.key === "Backspace" && !inputValue && addresses.length > 0) {
      removeAddress(addresses.length - 1);
    } else if (e.key === "ArrowDown" && showSuggestions) {
      e.preventDefault();
      setSelectedIdx((prev) => Math.min(prev + 1, suggestions.length - 1));
    } else if (e.key === "ArrowUp" && showSuggestions) {
      e.preventDefault();
      setSelectedIdx((prev) => Math.max(prev - 1, 0));
    } else if (e.key === "Escape") {
      setShowSuggestions(false);
    }
  };

  return (
    <div className="flex items-start gap-2">
      <span className="text-xs text-text-tertiary pt-1.5 w-8 shrink-0">
        {label}
      </span>
      <div className="flex-1 flex flex-wrap items-center gap-1 min-h-[32px] relative">
        {addresses.map((addr) => (
          <span
            key={addr}
            className="inline-flex items-center gap-1 bg-accent-light text-accent text-xs px-2 py-0.5 rounded-full"
          >
            {addr}
            <button
              type="button"
              onClick={() => onChange(addresses.filter((a) => a !== addr))}
              className="hover:text-danger text-[0.625rem] leading-none"
            >
              ×
            </button>
          </span>
        ))}
        <input
          ref={inputRef}
          type="text"
          value={inputValue}
          // biome-ignore lint/nursery/useExplicitType: inline callback
          onChange={(e) => handleInputChange(e.target.value)}
          onKeyDown={handleKeyDown}
          onBlur={() => {
            // Delay to allow click on suggestion
            if (blurTimerRef.current) clearTimeout(blurTimerRef.current);
            blurTimerRef.current = setTimeout(
              () => setShowSuggestions(false),
              150,
            );
            if (inputValue.trim()) addAddress(inputValue);
          }}
          placeholder={addresses.length === 0 ? resolvedPlaceholder : ""}
          aria-label={label}
          className="flex-1 min-w-[120px] bg-transparent text-sm text-text-primary outline-none placeholder:text-text-tertiary"
        />

        {/* Autocomplete dropdown */}
        {showSuggestions === true && (
          <div className="absolute top-full left-0 mt-1 w-full bg-bg-primary border border-border-primary rounded-md shadow-lg z-50 py-1">
            {suggestions.map((suggestion, i) => {
              const key =
                suggestion.kind === "group"
                  ? `group-${suggestion.data.id}`
                  : `contact-${suggestion.data.id}`;

              if (suggestion.kind === "group") {
                return (
                  <button
                    type="button"
                    key={key}
                    // biome-ignore lint/nursery/useExplicitType: inline callback
                    onMouseDown={(e) => e.preventDefault()}
                    onClick={() => void selectSuggestion(suggestion)}
                    className={`w-full text-left px-3 py-1.5 text-sm hover:bg-bg-hover ${
                      i === selectedIdx ? "bg-bg-hover" : ""
                    }`}
                  >
                    <div className="flex items-center gap-1.5 text-text-primary">
                      <Users
                        size={13}
                        className="text-text-tertiary shrink-0"
                      />
                      {suggestion.data.name}
                    </div>
                    <div className="text-xs text-text-tertiary ml-[21px]">
                      {t("groupMembers", {
                        count: suggestion.data.member_count,
                      })}
                    </div>
                  </button>
                );
              }

              return (
                <button
                  type="button"
                  key={key}
                  // biome-ignore lint/nursery/useExplicitType: inline callback
                  onMouseDown={(e) => e.preventDefault()}
                  onClick={() => void selectSuggestion(suggestion)}
                  className={`w-full text-left px-3 py-1.5 text-sm hover:bg-bg-hover ${
                    i === selectedIdx ? "bg-bg-hover" : ""
                  }`}
                >
                  <div className="text-text-primary">
                    {suggestion.data.display_name ?? suggestion.data.email}
                  </div>
                  {suggestion.data.display_name != null && (
                    <div className="text-xs text-text-tertiary">
                      {suggestion.data.email}
                    </div>
                  )}
                </button>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}
