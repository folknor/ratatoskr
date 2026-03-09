import type React from "react";
import { useTranslation } from "react-i18next";
import type { SecurityType } from "@/services/imap/autoDiscovery";
import type { FormState } from "./addImapAccountTypes";
import { inputClass, labelClass, selectClass } from "./addImapAccountTypes";

export interface AddImapAccountSmtpStepProps {
  form: FormState;
  updateForm: <K extends keyof FormState>(key: K, value: FormState[K]) => void;
  isOAuth: boolean;
  onSmtpSecurityChange: (security: SecurityType) => void;
}

export function AddImapAccountSmtpStep({
  form,
  updateForm,
  isOAuth,
  onSmtpSecurityChange,
}: AddImapAccountSmtpStepProps): React.ReactNode {
  const { t } = useTranslation("accounts");

  return (
    <div className="space-y-4">
      {isOAuth && (
        <p className="text-xs text-text-tertiary">{t("autoConfigured")}</p>
      )}
      <div>
        <label htmlFor="smtp-host" className={labelClass}>
          {t("smtpServer")}
        </label>
        <input
          id="smtp-host"
          type="text"
          value={form.smtpHost}
          onChange={(e: React.ChangeEvent<HTMLInputElement>): void =>
            updateForm("smtpHost", e.target.value)
          }
          placeholder={t("smtpServerPlaceholder")}
          className={inputClass}
        />
      </div>
      <div className="grid grid-cols-2 gap-3">
        <div>
          <label htmlFor="smtp-port" className={labelClass}>
            {t("port")}
          </label>
          <input
            id="smtp-port"
            type="number"
            value={form.smtpPort}
            onChange={(e: React.ChangeEvent<HTMLInputElement>): void =>
              updateForm("smtpPort", parseInt(e.target.value, 10) || 0)
            }
            className={inputClass}
          />
        </div>
        <div>
          <label htmlFor="smtp-security" className={labelClass}>
            {t("security")}
          </label>
          <select
            id="smtp-security"
            value={form.smtpSecurity}
            onChange={(e: React.ChangeEvent<HTMLSelectElement>): void =>
              onSmtpSecurityChange(e.target.value as SecurityType)
            }
            className={selectClass}
          >
            <option value="ssl">{t("sslTls")}</option>
            <option value="starttls">{t("starttls")}</option>
            <option value="none">{t("noneOption")}</option>
          </select>
        </div>
      </div>
      {!isOAuth && (
        <>
          <div className="flex items-center gap-2">
            <input
              id="smtp-same-password"
              type="checkbox"
              checked={form.samePassword}
              onChange={(e: React.ChangeEvent<HTMLInputElement>): void =>
                updateForm("samePassword", e.target.checked)
              }
              className="rounded border-border-primary text-accent focus:ring-accent"
            />
            <label
              htmlFor="smtp-same-password"
              className="text-sm text-text-secondary"
            >
              {t("samePasswordAsImap")}
            </label>
          </div>
          {!form.samePassword && (
            <div>
              <label htmlFor="smtp-password" className={labelClass}>
                {t("smtpPassword")}
              </label>
              <input
                id="smtp-password"
                type="password"
                value={form.smtpPassword}
                onChange={(e: React.ChangeEvent<HTMLInputElement>): void =>
                  updateForm("smtpPassword", e.target.value)
                }
                placeholder={t("smtpPasswordPlaceholder")}
                className={inputClass}
              />
            </div>
          )}
        </>
      )}
    </div>
  );
}
