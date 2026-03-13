import { invoke } from "@tauri-apps/api/core";

export interface DbContactGroup {
  id: string;
  name: string;
  member_count: number;
  created_at: string;
  updated_at: string;
}

export interface DbContactGroupMember {
  member_type: "email" | "group";
  member_value: string;
}

export async function createContactGroup(
  id: string,
  name: string,
): Promise<void> {
  return invoke("db_create_contact_group", { id, name });
}

export async function updateContactGroup(
  id: string,
  name: string,
): Promise<void> {
  return invoke("db_update_contact_group", { id, name });
}

export async function deleteContactGroup(id: string): Promise<void> {
  return invoke("db_delete_contact_group", { id });
}

export async function getAllContactGroups(): Promise<DbContactGroup[]> {
  return invoke("db_get_all_contact_groups");
}

export async function getContactGroup(id: string): Promise<DbContactGroup> {
  return invoke("db_get_contact_group", { id });
}

export async function getContactGroupMembers(
  groupId: string,
): Promise<DbContactGroupMember[]> {
  return invoke("db_get_contact_group_members", { groupId });
}

export async function addContactGroupMember(
  groupId: string,
  memberType: "email" | "group",
  memberValue: string,
): Promise<void> {
  return invoke("db_add_contact_group_member", {
    groupId,
    memberType,
    memberValue,
  });
}

export async function removeContactGroupMember(
  groupId: string,
  memberType: "email" | "group",
  memberValue: string,
): Promise<void> {
  return invoke("db_remove_contact_group_member", {
    groupId,
    memberType,
    memberValue,
  });
}

export async function searchContactGroups(
  query: string,
  limit: number = 10,
): Promise<DbContactGroup[]> {
  return invoke("db_search_contact_groups", { query, limit });
}

export async function expandContactGroup(groupId: string): Promise<string[]> {
  return invoke("db_expand_contact_group", { groupId });
}
