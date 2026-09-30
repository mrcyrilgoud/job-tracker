// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it } from "vitest";
import { EvidenceDisclosure } from "./EvidenceDisclosure";

describe("EvidenceDisclosure", () => {
  let root: ReturnType<typeof createRoot> | undefined;
  let container: HTMLDivElement | undefined;

  afterEach(() => {
    if (root) act(() => root?.unmount());
    container?.remove();
  });

  it("toggles aria-expanded and reveals the classification reason", () => {
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
    act(() => root?.render(<EvidenceDisclosure reason="Closed: listing returned 404" evidence={undefined} />));
    const button = container.querySelector("button") as HTMLButtonElement;
    expect(button.getAttribute("aria-expanded")).toBe("false");
    act(() => button.click());
    expect(button.getAttribute("aria-expanded")).toBe("true");
    expect(container.textContent).toContain("Closed: listing returned 404");
    act(() => button.click());
    expect(button.getAttribute("aria-expanded")).toBe("false");
  });
});
