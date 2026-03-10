import { Download, ExternalLink, Github, RefreshCw, Scale } from "lucide-react";
import type React from "react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import appIcon from "@/assets/icon.png";
import { Button } from "@/components/ui/Button";
import { InfoRow, Section } from "./SettingsShared";

export function SettingsAboutTab(): React.ReactNode {
  return (
    <>
      <DeveloperTab />
      <AboutTab />
    </>
  );
}

function DeveloperTab(): React.ReactNode {
  const { t } = useTranslation("settings");
  const [appVersion, setAppVersion] = useState("");
  const [tauriVersion, setTauriVersion] = useState("");
  const [webviewVersion, setWebviewVersion] = useState("");
  const [platformLabel, setPlatformLabel] = useState("...");
  const [checkingForUpdate, setCheckingForUpdate] = useState(false);
  const [updateVersion, setUpdateVersion] = useState<string | null>(null);
  const [updateCheckDone, setUpdateCheckDone] = useState(false);
  const [installingUpdate, setInstallingUpdate] = useState(false);

  useEffect(() => {
    async function load(): Promise<void> {
      const { getVersion, getTauriVersion } = await import(
        "@tauri-apps/api/app"
      );
      setAppVersion(await getVersion());
      setTauriVersion(await getTauriVersion());

      // Extract WebView version from user agent
      const ua = navigator.userAgent;
      const edgMatch = /Edg\/(\S+)/.exec(ua);
      const chromeMatch = /Chrome\/(\S+)/.exec(ua);
      const webkitMatch = /AppleWebKit\/(\S+)/.exec(ua);
      setWebviewVersion(
        edgMatch?.[1] ?? chromeMatch?.[1] ?? webkitMatch?.[1] ?? "Unknown",
      );

      // Detect platform via Tauri OS plugin (reliable native arch detection)
      const { platform, arch } = await import("@tauri-apps/plugin-os");
      const p = platform();
      const a = arch();
      const archLabel =
        a === "aarch64" || a === "arm" ? "ARM" : a === "x86_64" ? "x64" : a;
      if (p === "macos") {
        setPlatformLabel(
          a === "aarch64" ? "macOS (Apple Silicon)" : `macOS (${archLabel})`,
        );
      } else if (p === "windows") {
        setPlatformLabel(`Windows (${archLabel})`);
      } else if (p === "linux") {
        setPlatformLabel(`Linux (${archLabel})`);
      } else {
        setPlatformLabel(`${p} (${archLabel})`);
      }

      // Check if there's already a known update
      const { getAvailableUpdate } = await import("@/services/updateManager");
      const existing = getAvailableUpdate();
      if (existing) setUpdateVersion(existing.version);
    }
    void load();
  }, []);

  const handleCheckForUpdate = async (): Promise<void> => {
    setCheckingForUpdate(true);
    setUpdateCheckDone(false);
    setUpdateVersion(null);
    try {
      const { checkForUpdateNow } = await import("@/services/updateManager");
      const result = await checkForUpdateNow();
      if (result) {
        setUpdateVersion(result.version);
      } else {
        setUpdateCheckDone(true);
      }
    } catch (err) {
      console.error("Update check failed:", err);
      setUpdateCheckDone(true);
    } finally {
      setCheckingForUpdate(false);
    }
  };

  const handleInstallUpdate = async (): Promise<void> => {
    setInstallingUpdate(true);
    try {
      const { installUpdate } = await import("@/services/updateManager");
      await installUpdate();
    } catch (err) {
      console.error("Update install failed:", err);
      setInstallingUpdate(false);
    }
  };

  return (
    <>
      <Section title={t("appInfo")}>
        <InfoRow label={t("appVersion")} value={appVersion || "..."} />
        <InfoRow label={t("tauriVersion")} value={tauriVersion || "..."} />
        <InfoRow label={t("webviewVersion")} value={webviewVersion || "..."} />
        <InfoRow label={t("platform")} value={platformLabel} />
      </Section>

      <Section title={t("updatesSection")}>
        <div className="flex items-center justify-between">
          <div>
            <span className="text-sm text-text-secondary">
              {t("softwareUpdates")}
            </span>
            {updateVersion != null && (
              <p className="text-xs text-accent mt-0.5">
                v{updateVersion} {t("available")}
              </p>
            )}
            {updateCheckDone && !updateVersion && (
              <p className="text-xs text-success mt-0.5">{t("upToDate")}</p>
            )}
          </div>
          <div className="flex items-center gap-2">
            {updateVersion ? (
              <Button
                variant="primary"
                size="md"
                icon={<Download size={14} />}
                onClick={handleInstallUpdate}
                disabled={installingUpdate}
              >
                {installingUpdate ? t("updating") : t("updateAndRestart")}
              </Button>
            ) : (
              <Button
                variant="secondary"
                size="md"
                icon={
                  <RefreshCw
                    size={14}
                    className={checkingForUpdate ? "animate-spin" : ""}
                  />
                }
                onClick={handleCheckForUpdate}
                disabled={checkingForUpdate}
                className="bg-bg-tertiary text-text-primary border border-border-primary"
              >
                {checkingForUpdate ? t("checking") : t("checkForUpdates")}
              </Button>
            )}
          </div>
        </div>
      </Section>

      <Section title={t("developerTools")}>
        <div className="flex items-center justify-between">
          <div>
            <span className="text-sm text-text-secondary">
              {t("openDevTools")}
            </span>
            <p className="text-xs text-text-tertiary mt-0.5">
              {t("openDevToolsDescription")}
            </p>
          </div>
          <Button
            variant="secondary"
            size="md"
            onClick={async (): Promise<void> => {
              const { invoke } = await import("@tauri-apps/api/core");
              await invoke("open_devtools");
            }}
            className="bg-bg-tertiary text-text-primary border border-border-primary"
          >
            {t("openDevTools")}
          </Button>
        </div>
      </Section>
    </>
  );
}

function AboutTab(): React.ReactNode {
  const { t } = useTranslation("settings");
  const [appVersion, setAppVersion] = useState("");

  useEffect(() => {
    void import("@tauri-apps/api/app").then(({ getVersion }) =>
      getVersion().then(setAppVersion),
    );
  }, []);

  const openExternal = async (url: string): Promise<void> => {
    const { openUrl } = await import("@tauri-apps/plugin-opener");
    await openUrl(url);
  };

  return (
    <>
      <Section title={t("ratatoskrMail")}>
        <div className="flex items-center gap-3 mb-2">
          <img src={appIcon} alt="Ratatoskr" className="w-12 h-12 rounded-xl" />
          <div>
            <h3 className="text-base font-semibold text-text-primary">
              Ratatoskr
            </h3>
            <p className="text-sm text-text-tertiary">
              {appVersion ? `${t("version")} ${appVersion}` : t("loading")}
            </p>
          </div>
        </div>
        <p className="text-sm text-text-secondary leading-relaxed">
          {t("aboutDescription")}
        </p>
      </Section>

      <Section title={t("links")}>
        <div className="space-y-1">
          <button
            type="button"
            onClick={(): void =>
              void openExternal("https://github.com/folknor/ratatoskr")
            }
            className="flex items-center gap-3 w-full px-4 py-2.5 rounded-lg bg-bg-secondary hover:bg-bg-hover transition-colors text-left"
          >
            <Github size={16} className="text-text-tertiary shrink-0" />
            <div className="min-w-0 flex-1">
              <span className="text-sm text-text-primary">
                {t("githubRepository")}
              </span>
              <p className="text-xs text-text-tertiary">folknor/ratatoskr</p>
            </div>
            <ExternalLink size={14} className="text-text-tertiary shrink-0" />
          </button>
        </div>
      </Section>

      <Section title={t("license")}>
        <div className="px-4 py-3 bg-bg-secondary rounded-lg">
          <div className="flex items-center gap-2 mb-2">
            <Scale size={15} className="text-text-tertiary" />
            <span className="text-sm font-medium text-text-primary">
              {t("apacheLicense")}
            </span>
          </div>
          <p className="text-xs text-text-secondary leading-relaxed mb-3">
            {t("licenseDescription")}{" "}
            <button
              type="button"
              onClick={(): void =>
                void openExternal("https://www.apache.org/licenses/LICENSE-2.0")
              }
              className="text-accent hover:text-accent-hover transition-colors"
            >
              apache.org/licenses/LICENSE-2.0
            </button>
          </p>
          <p className="text-xs text-text-tertiary leading-relaxed">
            {t("copyright")}
          </p>
        </div>
      </Section>
    </>
  );
}
