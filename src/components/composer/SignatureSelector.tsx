import type React from "react";
import { useEffect, useState } from "react";
import { type DbSignature, getSignaturesForAccount } from "@/core/composer";
import { useAccountStore } from "@/stores/accountStore";
import { useComposerStore } from "@/stores/composerStore";

export function SignatureSelector(): React.ReactNode {
  const activeAccountId = useAccountStore((s) => s.activeAccountId);
  const isOpen = useComposerStore((s) => s.isOpen);
  const signatureId = useComposerStore((s) => s.signatureId);
  const setSignatureHtml = useComposerStore((s) => s.setSignatureHtml);
  const setSignatureId = useComposerStore((s) => s.setSignatureId);
  const [signatures, setSignatures] = useState<DbSignature[]>([]);

  useEffect(() => {
    if (!(isOpen && activeAccountId)) return;
    let cancelled = false;
    getSignaturesForAccount(activeAccountId).then((sigs) => {
      if (!cancelled) setSignatures(sigs);
    });
    return () => {
      cancelled = true;
    };
  }, [isOpen, activeAccountId]);

  if (signatures.length === 0) return null;

  const handleChange = (id: string): void => {
    if (id === "") {
      setSignatureId(null);
      setSignatureHtml("");
      return;
    }
    const sig = signatures.find((s) => s.id === id);
    if (sig) {
      setSignatureId(sig.id);
      setSignatureHtml(sig.body_html);
    }
  };

  return (
    <select
      value={signatureId ?? ""}
      // biome-ignore lint/nursery/useExplicitType: inline callback
      onChange={(e) => handleChange(e.target.value)}
      className="text-[0.625rem] bg-bg-tertiary text-text-secondary border border-border-primary rounded px-1.5 py-0.5"
    >
      <option value="">No signature</option>
      {signatures.map((sig) => (
        <option key={sig.id} value={sig.id}>
          {sig.name}
        </option>
      ))}
    </select>
  );
}
