import {
  resolveContextMenuTargets,
  resolveKeyboardTargets,
} from "./multiSelectTargets";

describe("resolveContextMenuTargets", () => {
  it("returns just the right-clicked thread when no multi-selection", () => {
    const result = resolveContextMenuTargets("t1", new Set());
    expect(result.targetIds).toEqual(["t1"]);
    expect(result.isMulti).toBe(false);
  });

  it("returns just the right-clicked thread when it is not in multi-selection", () => {
    const result = resolveContextMenuTargets("t3", new Set(["t1", "t2"]));
    expect(result.targetIds).toEqual(["t3"]);
    expect(result.isMulti).toBe(false);
  });

  it("returns just the right-clicked thread when it is the only one selected", () => {
    const result = resolveContextMenuTargets("t1", new Set(["t1"]));
    expect(result.targetIds).toEqual(["t1"]);
    expect(result.isMulti).toBe(false);
  });

  it("returns all selected threads when right-clicked thread is in multi-selection", () => {
    const result = resolveContextMenuTargets("t2", new Set(["t1", "t2", "t3"]));
    expect(result.targetIds).toHaveLength(3);
    expect(new Set(result.targetIds)).toEqual(new Set(["t1", "t2", "t3"]));
    expect(result.isMulti).toBe(true);
  });
});

describe("resolveKeyboardTargets", () => {
  it("returns multi-selected threads when present", () => {
    const result = resolveKeyboardTargets(new Set(["t1", "t2"]), "t3");
    expect(result).toHaveLength(2);
    expect(new Set(result)).toEqual(new Set(["t1", "t2"]));
  });

  it("returns focused thread when no multi-selection", () => {
    const result = resolveKeyboardTargets(new Set(), "t1");
    expect(result).toEqual(["t1"]);
  });

  it("returns empty array when no selection and no focused thread", () => {
    expect(resolveKeyboardTargets(new Set(), null)).toEqual([]);
    expect(resolveKeyboardTargets(new Set(), undefined)).toEqual([]);
  });

  it("prefers multi-selection over focused thread", () => {
    const result = resolveKeyboardTargets(new Set(["t1"]), "t2");
    expect(result).toEqual(["t1"]);
  });
});
