import { Filter, FolderSearch } from "lucide-react";
import type React from "react";
import { useTranslation } from "react-i18next";
import { EmptyState } from "../ui/EmptyState";
import {
  GenericEmptyIllustration,
  InboxClearIllustration,
  NoAccountIllustration,
  NoSearchResultsIllustration,
} from "../ui/illustrations";

export function EmptyStateForContext({
  searchQuery,
  activeAccountId,
  activeLabel,
  readFilter,
  activeCategory,
}: {
  searchQuery: string | null;
  activeAccountId: string | null;
  activeLabel: string;
  readFilter: string;
  activeCategory: string;
}): React.ReactNode {
  const { t } = useTranslation("email");

  if (searchQuery) {
    return (
      <EmptyState
        illustration={NoSearchResultsIllustration}
        title={t("noSearchResults")}
        subtitle={t("tryDifferentSearch")}
      />
    );
  }
  if (readFilter !== "all") {
    return (
      <EmptyState
        icon={Filter}
        title={t("noFilteredEmails", { filter: readFilter })}
        subtitle={t("tryChangingFilter")}
      />
    );
  }
  if (!activeAccountId) {
    return (
      <EmptyState
        illustration={NoAccountIllustration}
        title={t("noAccountConnected")}
        subtitle={t("addAccountToStart")}
      />
    );
  }

  switch (activeLabel) {
    case "inbox":
      if (activeCategory !== "All") {
        const categoryMessages: Record<
          string,
          { title: string; subtitle: string }
        > = {
          Primary: {
            title: t("primaryClear"),
            subtitle: t("noImportantConversations"),
          },
          Updates: { title: t("noUpdates"), subtitle: t("updatesDescription") },
          Promotions: {
            title: t("noPromotions"),
            subtitle: t("promotionsDescription"),
          },
          Social: {
            title: t("noSocialEmails"),
            subtitle: t("socialDescription"),
          },
          Newsletters: {
            title: t("noNewsletters"),
            subtitle: t("newslettersDescription"),
          },
        };
        const msg = categoryMessages[activeCategory];
        if (msg)
          return (
            <EmptyState
              illustration={InboxClearIllustration}
              title={msg.title}
              subtitle={msg.subtitle}
            />
          );
      }
      return (
        <EmptyState
          illustration={InboxClearIllustration}
          title={t("allCaughtUp")}
          subtitle={t("noNewConversations")}
        />
      );
    case "starred":
      return (
        <EmptyState
          illustration={GenericEmptyIllustration}
          title={t("noStarred")}
          subtitle={t("starToFind")}
        />
      );
    case "snoozed":
      return (
        <EmptyState
          illustration={GenericEmptyIllustration}
          title={t("noSnoozed")}
          subtitle={t("snoozedAppearHere")}
        />
      );
    case "sent":
      return (
        <EmptyState
          illustration={GenericEmptyIllustration}
          title={t("noSentMessages")}
        />
      );
    case "drafts":
      return (
        <EmptyState
          illustration={GenericEmptyIllustration}
          title={t("noDrafts")}
        />
      );
    case "trash":
      return (
        <EmptyState
          illustration={GenericEmptyIllustration}
          title={t("trashEmpty")}
        />
      );
    case "spam":
      return (
        <EmptyState
          illustration={GenericEmptyIllustration}
          title={t("noSpam")}
          subtitle={t("lookingGood")}
        />
      );
    case "all":
      return (
        <EmptyState
          illustration={GenericEmptyIllustration}
          title={t("noEmails")}
        />
      );
    default:
      if (activeLabel.startsWith("smart-folder:")) {
        return (
          <EmptyState
            icon={FolderSearch}
            title={t("noSmartFolderMatch")}
            subtitle={t("adjustSmartFolder")}
          />
        );
      }
      return (
        <EmptyState
          illustration={GenericEmptyIllustration}
          title={t("nothingHere")}
          subtitle={t("noLabelConversations")}
        />
      );
  }
}
