// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MemoryRouter } from "react-router-dom";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { SidebarLink, type SidebarLinkProps } from "./SidebarLink";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

describe("SidebarLink trailing slot", () => {
  let root: Root;
  let host: HTMLDivElement;

  beforeEach(() => {
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
  });

  afterEach(() => {
    act(() => root.unmount());
    host.remove();
  });

  function render(props: SidebarLinkProps): HTMLAnchorElement {
    act(() => {
      root.render(createElement(MemoryRouter, null, createElement(SidebarLink, props)));
    });
    return host.querySelector("a")!;
  }

  it("renders a plain count when no annotation is given", () => {
    const link = render({ to: "/x", label: "AKASA", count: 3 });
    expect(link.textContent).toBe("AKASA3");
    expect(link.querySelector(".sr-only")).toBeNull();
  });

  it("renders annotation text hidden from AT plus an sr-only label", () => {
    const link = render({ to: "/x", label: "AKASA", annotation: { text: "3", label: "3 wishlist jobs" } });
    const hidden = link.querySelector('[aria-hidden="true"]');
    const sr = link.querySelector(".sr-only");
    expect(hidden?.textContent).toBe("3");
    expect(sr?.textContent).toBe("3 wishlist jobs");
    expect(hidden?.parentElement?.className).toContain("font-display");
  });

  it("annotation takes precedence over count", () => {
    const link = render({ to: "/x", label: "AKASA", count: 9, annotation: { text: "Mar 4", label: "Last applied Mar 4" } });
    expect(link.textContent).toBe("AKASAMar 4Last applied Mar 4");
    expect(link.textContent).not.toContain("9");
  });

  it("renders no trailing slot without count or annotation", () => {
    const link = render({ to: "/x", label: "AKASA" });
    expect(link.textContent).toBe("AKASA");
  });
});
