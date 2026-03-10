import { WifiOff } from "lucide-react";
import type React from "react";
import { useTranslation } from "react-i18next";
import { useSyncStateStore } from "@/stores/syncStateStore";

export function OfflineBanner(): React.ReactNode {
  const { t } = useTranslation();
  const isOnline = useSyncStateStore((s) => s.isOnline);

  if (isOnline) return null;

  return (
    <div className="fixed top-8 left-0 right-0 z-50 flex items-center justify-center gap-2 bg-warning/90 text-white text-xs px-4 py-1.5 backdrop-blur-sm">
      <WifiOff size={14} />
      <span>{t("offline")}</span>
    </div>
  );
}
