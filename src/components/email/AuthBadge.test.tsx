import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { AuthResult } from "@/services/gmail/authParser";
import { AuthBadge } from "./AuthBadge";

function makeAuthResults(aggregate: AuthResult["aggregate"]): string {
  const result: AuthResult = {
    spf: { result: aggregate === "pass" ? "pass" : "fail", detail: null },
    dkim: { result: aggregate === "pass" ? "pass" : "fail", detail: null },
    dmarc: { result: aggregate === "pass" ? "pass" : "fail", detail: null },
    aggregate,
  };
  return JSON.stringify(result);
}

describe("AuthBadge", () => {
  it("should render ShieldCheck for pass aggregate", () => {
    const { container } = render(
      <AuthBadge authResults={makeAuthResults("pass")} />,
    );

    const badge = container.querySelector(
      "[aria-label='Authentication passed']",
    );
    expect(badge).toBeInTheDocument();
    expect(badge?.className).toContain("text-success");
  });

  it("should render ShieldX for fail aggregate", () => {
    const { container } = render(
      <AuthBadge authResults={makeAuthResults("fail")} />,
    );

    const badge = container.querySelector(
      "[aria-label='Authentication failed']",
    );
    expect(badge).toBeInTheDocument();
    expect(badge?.className).toContain("text-danger");
  });

  it("should render nothing for null authResults", () => {
    const { container } = render(<AuthBadge authResults={null} />);

    expect(container.innerHTML).toBe("");
  });

  it("should render ShieldAlert for warning aggregate", () => {
    const { container } = render(
      <AuthBadge authResults={makeAuthResults("warning")} />,
    );

    const badge = container.querySelector(
      "[aria-label='Authentication warning']",
    );
    expect(badge).toBeInTheDocument();
    expect(badge?.className).toContain("text-warning");
  });

  it("should render ShieldQuestion for unknown aggregate", () => {
    const { container } = render(
      <AuthBadge authResults={makeAuthResults("unknown")} />,
    );

    const badge = container.querySelector(
      "[aria-label='Authentication unknown']",
    );
    expect(badge).toBeInTheDocument();
    expect(badge?.className).toContain("text-text-tertiary");
  });
});
