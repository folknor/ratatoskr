import type React from "react";
import { useTranslation } from "react-i18next";
import type { SecurityType } from "@/core/accounts";
import type { FormState } from "./addImapAccountTypes";
import { inputClass, labelClass, selectClass } from "./addImapAccountTypes";

export interface AddImapAccountImapStepProps {
  form: FormState;
  updateForm: <K extends keyof FormState>(key: K, value: FormState[K]) => void;
  isOAuth: boolean;
  onImapSecurityChange: (security: SecurityType) => void;
}

export function AddImapAccountImapStep({
  form,
  updateForm,
  isOAuth,
  onImapSecurityChange,
}: AddImapAccountImapStepProps): React.ReactNode {
  const { t } = useTranslation("accounts");

  return (
    <div className="space-y-4">
      {isOAuth && (
        <p className="text-xs text-text-tertiary">{t("autoConfigured")}</p>
      )}
      <div>
        <label htmlFor="imap-host" className={labelClass}>
          {t("imapServer")}
        </label>
        <input
          id="imap-host"
          type="text"
          value={form.imapHost}
          onChange={(e: React.ChangeEvent<HTMLInputElement>): void =>
            updateForm("imapHost", e.target.value)
          }
          placeholder={t("imapServerPlaceholder")}
          className={inputClass}
        />
      </div>
      <div className="grid grid-cols-2 gap-3">
        <div>
          <label htmlFor="imap-port" className={labelClass}>
            {t("port")}
          </label>
          <input
            id="imap-port"
            type="number"
            value={form.imapPort}
            onChange={(e: React.ChangeEvent<HTMLInputElement>): void =>
              updateForm("imapPort", parseInt(e.target.value, 10) || 0)
            }
            className={inputClass}
          />
        </div>
        <div>
          <label htmlFor="imap-security" className={labelClass}>
            {t("security")}
          </label>
          <select
            id="imap-security"
            value={form.imapSecurity}
            onChange={(e: React.ChangeEvent<HTMLSelectElement>): void =>
              onImapSecurityChange(e.target.value as SecurityType)
            }
            className={selectClass}
          >
            <option value="ssl">{t("sslTls")}</option>
            <option value="starttls">{t("starttls")}</option>
            <option value="none">{t("noneOption")}</option>
          </select>
        </div>
      </div>
      <div className="flex items-center gap-2">
        <input
          id="accept-invalid-certs"
          type="checkbox"
          checked={form.acceptInvalidCerts}
          onChange={(e: React.ChangeEvent<HTMLInputElement>): void =>
            updateForm("acceptInvalidCerts", e.target.checked)
          }
          className="rounded border-border-primary text-accent focus:ring-accent"
        />
        <label
          htmlFor="accept-invalid-certs"
          className="text-sm text-text-secondary"
        >
          {t("selfSignedCerts")}
        </label>
      </div>
      <p className="text-xs text-text-tertiary -mt-2 ml-6">
        {t("selfSignedHelp")}
      </p>
    </div>
  );
}
