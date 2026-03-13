import {
  Check,
  ChevronDown,
  ChevronRight,
  Pencil,
  Plus,
  Search,
  Trash2,
  Users,
  X,
} from "lucide-react";
import type React from "react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  addContactGroupMember,
  createContactGroup,
  type DbContactGroup,
  type DbContactGroupMember,
  deleteContactGroup,
  getAllContactGroups,
  getContactGroupMembers,
  removeContactGroupMember,
  updateContactGroup,
} from "@/core/queries";

export function GroupEditor(): React.ReactNode {
  const { t } = useTranslation("settings");
  const [groups, setGroups] = useState<DbContactGroup[]>([]);
  const [search, setSearch] = useState("");
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [members, setMembers] = useState<DbContactGroupMember[]>([]);
  const [newGroupName, setNewGroupName] = useState("");
  const [showNewGroup, setShowNewGroup] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editName, setEditName] = useState("");
  const [newMemberValue, setNewMemberValue] = useState("");
  const [newMemberType, setNewMemberType] = useState<"email" | "group">(
    "email",
  );

  const loadGroups = useCallback(async (): Promise<void> => {
    const all = await getAllContactGroups();
    setGroups(all);
  }, []);

  useEffect(() => {
    void loadGroups();
  }, [loadGroups]);

  const loadMembers = useCallback(async (groupId: string): Promise<void> => {
    const m = await getContactGroupMembers(groupId);
    setMembers(m);
  }, []);

  const filtered = useMemo((): DbContactGroup[] => {
    if (!search) return groups;
    const q = search.toLowerCase();
    return groups.filter((g) => g.name.toLowerCase().includes(q));
  }, [groups, search]);

  const handleCreateGroup = async (): Promise<void> => {
    const name = newGroupName.trim();
    if (!name) return;
    const id = crypto.randomUUID();
    await createContactGroup(id, name);
    setNewGroupName("");
    setShowNewGroup(false);
    await loadGroups();
  };

  const handleDeleteGroup = async (id: string): Promise<void> => {
    await deleteContactGroup(id);
    if (expandedId === id) {
      setExpandedId(null);
      setMembers([]);
    }
    await loadGroups();
  };

  const handleToggleExpand = async (id: string): Promise<void> => {
    if (expandedId === id) {
      setExpandedId(null);
      setMembers([]);
    } else {
      setExpandedId(id);
      await loadMembers(id);
    }
  };

  const handleSaveEdit = async (): Promise<void> => {
    if (!editingId) return;
    await updateContactGroup(editingId, editName);
    setEditingId(null);
    await loadGroups();
  };

  const handleAddMember = async (): Promise<void> => {
    if (!(expandedId && newMemberValue.trim())) return;
    await addContactGroupMember(
      expandedId,
      newMemberType,
      newMemberValue.trim(),
    );
    setNewMemberValue("");
    await loadMembers(expandedId);
    await loadGroups();
  };

  const handleRemoveMember = async (
    memberType: "email" | "group",
    memberValue: string,
  ): Promise<void> => {
    if (!expandedId) return;
    await removeContactGroupMember(expandedId, memberType, memberValue);
    await loadMembers(expandedId);
    await loadGroups();
  };

  return (
    <div className="space-y-3">
      <div className="flex gap-2">
        <div className="relative flex-1">
          <Search
            size={14}
            className="absolute left-2.5 top-1/2 -translate-y-1/2 text-text-tertiary"
          />
          <input
            type="text"
            value={search}
            onChange={(e: React.ChangeEvent<HTMLInputElement>): void =>
              setSearch(e.target.value)
            }
            placeholder={t("groupEditor.searchPlaceholder")}
            className="w-full pl-8 pr-3 py-1.5 bg-bg-tertiary border border-border-primary rounded text-sm text-text-primary outline-none focus:border-accent"
          />
        </div>
        <button
          type="button"
          onClick={(): void => setShowNewGroup(true)}
          className="flex items-center gap-1 px-2.5 py-1.5 bg-accent text-white text-xs rounded hover:bg-accent/90"
        >
          <Plus size={13} />
          {t("groupEditor.newGroup")}
        </button>
      </div>

      {showNewGroup === true && (
        <div className="flex items-center gap-2 px-2 py-1.5 bg-bg-hover rounded">
          <input
            type="text"
            value={newGroupName}
            onChange={(e: React.ChangeEvent<HTMLInputElement>): void =>
              setNewGroupName(e.target.value)
            }
            onKeyDown={(e: React.KeyboardEvent<HTMLInputElement>): void => {
              if (e.key === "Enter") void handleCreateGroup();
              if (e.key === "Escape") setShowNewGroup(false);
            }}
            placeholder={t("groupEditor.groupName")}
            className="flex-1 min-w-0 px-2 py-0.5 bg-bg-tertiary border border-border-primary rounded text-sm text-text-primary outline-none focus:border-accent"
            // biome-ignore lint/a11y/noAutofocus: focus new input for UX
            autoFocus
          />
          <button
            type="button"
            onClick={(): void => void handleCreateGroup()}
            className="p-1 text-success hover:bg-bg-hover rounded"
          >
            <Check size={14} />
          </button>
          <button
            type="button"
            onClick={(): void => setShowNewGroup(false)}
            className="p-1 text-text-tertiary hover:text-text-primary rounded"
          >
            <X size={14} />
          </button>
        </div>
      )}

      {filtered.length === 0 ? (
        <p className="text-sm text-text-tertiary py-2">
          {search ? t("groupEditor.noMatching") : t("groupEditor.noGroups")}
        </p>
      ) : (
        <div className="space-y-1 max-h-[400px] overflow-y-auto">
          {filtered.map((group) => (
            <div key={group.id}>
              <div className="flex items-center justify-between py-1.5 px-2 rounded hover:bg-bg-hover group">
                {editingId === group.id ? (
                  <div className="flex items-center gap-2 flex-1 min-w-0">
                    <input
                      type="text"
                      value={editName}
                      onChange={(
                        e: React.ChangeEvent<HTMLInputElement>,
                      ): void => setEditName(e.target.value)}
                      onKeyDown={(
                        e: React.KeyboardEvent<HTMLInputElement>,
                      ): void => {
                        if (e.key === "Enter") void handleSaveEdit();
                        if (e.key === "Escape") setEditingId(null);
                      }}
                      className="flex-1 min-w-0 px-2 py-0.5 bg-bg-tertiary border border-border-primary rounded text-sm text-text-primary outline-none focus:border-accent"
                      // biome-ignore lint/a11y/noAutofocus: focus new input for UX
                      autoFocus
                    />
                    <button
                      type="button"
                      onClick={(): void => void handleSaveEdit()}
                      className="p-1 text-success hover:bg-bg-hover rounded"
                    >
                      <Check size={14} />
                    </button>
                    <button
                      type="button"
                      onClick={(): void => setEditingId(null)}
                      className="p-1 text-text-tertiary hover:text-text-primary rounded"
                    >
                      <X size={14} />
                    </button>
                  </div>
                ) : (
                  <>
                    <button
                      type="button"
                      onClick={(): void => void handleToggleExpand(group.id)}
                      className="flex items-center gap-1.5 min-w-0 flex-1 text-left"
                    >
                      {expandedId === group.id ? (
                        <ChevronDown
                          size={14}
                          className="text-text-tertiary shrink-0"
                        />
                      ) : (
                        <ChevronRight
                          size={14}
                          className="text-text-tertiary shrink-0"
                        />
                      )}
                      <Users
                        size={13}
                        className="text-text-tertiary shrink-0"
                      />
                      <span className="text-sm text-text-primary truncate">
                        {group.name}
                      </span>
                      <span className="text-xs text-text-tertiary ml-1">
                        ({group.member_count})
                      </span>
                    </button>
                    <div className="flex items-center gap-1">
                      <button
                        type="button"
                        onClick={(): void => {
                          setEditingId(group.id);
                          setEditName(group.name);
                        }}
                        className="p-1 text-text-tertiary hover:text-text-primary opacity-0 group-hover:opacity-100 transition-opacity"
                        title={t("groupEditor.editName")}
                      >
                        <Pencil size={13} />
                      </button>
                      <button
                        type="button"
                        onClick={(): void => void handleDeleteGroup(group.id)}
                        className="p-1 text-text-tertiary hover:text-danger opacity-0 group-hover:opacity-100 transition-opacity"
                        title={t("groupEditor.deleteGroup")}
                      >
                        <Trash2 size={13} />
                      </button>
                    </div>
                  </>
                )}
              </div>

              {/* Expanded member list */}
              {expandedId === group.id && (
                <div className="ml-8 mt-1 mb-2 space-y-1">
                  {members.map((member) => (
                    <div
                      key={`${member.member_type}-${member.member_value}`}
                      className="flex items-center justify-between py-1 px-2 rounded hover:bg-bg-hover text-sm group/member"
                    >
                      <span className="text-text-primary truncate">
                        {member.member_type === "group" ? (
                          <span className="flex items-center gap-1">
                            <Users size={12} className="text-text-tertiary" />
                            {groups.find((g) => g.id === member.member_value)
                              ?.name ?? member.member_value}
                          </span>
                        ) : (
                          member.member_value
                        )}
                      </span>
                      <button
                        type="button"
                        onClick={(): void =>
                          void handleRemoveMember(
                            member.member_type,
                            member.member_value,
                          )
                        }
                        className="p-0.5 text-text-tertiary hover:text-danger opacity-0 group-hover/member:opacity-100 transition-opacity"
                      >
                        <X size={12} />
                      </button>
                    </div>
                  ))}

                  {/* Add member input */}
                  <div className="flex items-center gap-2 pt-1">
                    <select
                      value={newMemberType}
                      onChange={(
                        e: React.ChangeEvent<HTMLSelectElement>,
                      ): void =>
                        setNewMemberType(e.target.value as "email" | "group")
                      }
                      className="px-1.5 py-1 bg-bg-tertiary border border-border-primary rounded text-xs text-text-primary outline-none"
                    >
                      <option value="email">Email</option>
                      <option value="group">Group</option>
                    </select>
                    {newMemberType === "email" ? (
                      <input
                        type="text"
                        value={newMemberValue}
                        onChange={(
                          e: React.ChangeEvent<HTMLInputElement>,
                        ): void => setNewMemberValue(e.target.value)}
                        onKeyDown={(
                          e: React.KeyboardEvent<HTMLInputElement>,
                        ): void => {
                          if (e.key === "Enter") void handleAddMember();
                        }}
                        placeholder={t("groupEditor.emailPlaceholder")}
                        className="flex-1 min-w-0 px-2 py-1 bg-bg-tertiary border border-border-primary rounded text-xs text-text-primary outline-none focus:border-accent"
                      />
                    ) : (
                      <select
                        value={newMemberValue}
                        onChange={(
                          e: React.ChangeEvent<HTMLSelectElement>,
                        ): void => setNewMemberValue(e.target.value)}
                        className="flex-1 min-w-0 px-2 py-1 bg-bg-tertiary border border-border-primary rounded text-xs text-text-primary outline-none"
                      >
                        <option value="">{t("groupEditor.selectGroup")}</option>
                        {groups
                          .filter((g) => g.id !== group.id)
                          .map((g) => (
                            <option key={g.id} value={g.id}>
                              {g.name}
                            </option>
                          ))}
                      </select>
                    )}
                    <button
                      type="button"
                      onClick={(): void => void handleAddMember()}
                      className="p-1 text-accent hover:text-accent/80"
                      title={t("groupEditor.addMember")}
                    >
                      <Plus size={14} />
                    </button>
                  </div>
                </div>
              )}
            </div>
          ))}
        </div>
      )}

      <p className="text-xs text-text-tertiary">
        {t("groupEditor.group", { count: groups.length })}{" "}
        {t("contactEditor.total")}
      </p>
    </div>
  );
}
