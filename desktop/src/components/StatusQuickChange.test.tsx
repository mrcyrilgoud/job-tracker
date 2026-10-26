import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { StatusQuickChange } from "./StatusQuickChange";
import { jobStatuses } from "@/lib/schema";

describe("StatusQuickChange", () => {
  const mockOnStatusChange = vi.fn();

  beforeEach(() => {
    mockOnStatusChange.mockClear();
  });

  it("renders with current status", () => {
    render(
      <StatusQuickChange
        currentStatus="wishlist"
        onStatusChange={mockOnStatusChange}
      />
    );

    expect(screen.getByText("Wishlist")).toBeInTheDocument();
  });

  it("opens dropdown when clicked", () => {
    render(
      <StatusQuickChange
        currentStatus="applied"
        onStatusChange={mockOnStatusChange}
      />
    );

    const button = screen.getByRole("button", { name: /change status/i });
    fireEvent.click(button);

    expect(screen.getByRole("menu")).toBeInTheDocument();
    expect(screen.getByText("Change Status")).toBeInTheDocument();
  });

  it("shows all status options in dropdown", () => {
    render(
      <StatusQuickChange
        currentStatus="interviewing"
        onStatusChange={mockOnStatusChange}
      />
    );

    const button = screen.getByRole("button", { name: /change status/i });
    fireEvent.click(button);

    jobStatuses.forEach((status) => {
      expect(screen.getByText(new RegExp(status, "i"))).toBeInTheDocument();
    });
  });

  it("calls onStatusChange when a status is selected", async () => {
    render(
      <StatusQuickChange
        currentStatus="wishlist"
        onStatusChange={mockOnStatusChange}
      />
    );

    const button = screen.getByRole("button", { name: /change status/i });
    fireEvent.click(button);

    const appliedOption = screen.getByRole("menuitem", { name: /applied/i });
    fireEvent.click(appliedOption);

    await waitFor(() => {
      expect(mockOnStatusChange).toHaveBeenCalledWith("applied");
    });
  });

  it("shows success feedback after status change", async () => {
    render(
      <StatusQuickChange
        currentStatus="wishlist"
        onStatusChange={mockOnStatusChange}
      />
    );

    const button = screen.getByRole("button", { name: /change status/i });
    fireEvent.click(button);

    const appliedOption = screen.getByRole("menuitem", { name: /applied/i });
    fireEvent.click(appliedOption);

    // Should show success feedback
    await waitFor(() => {
      expect(screen.getByText("Saved!")).toBeInTheDocument();
    });
  });

  it("closes dropdown when clicking outside", async () => {
    render(
      <div>
        <StatusQuickChange
          currentStatus="wishlist"
          onStatusChange={mockOnStatusChange}
        />
        <div data-testid="outside">Outside element</div>
      </div>
    );

    const button = screen.getByRole("button", { name: /change status/i });
    fireEvent.click(button);

    expect(screen.getByRole("menu")).toBeInTheDocument();

    // Click outside
    fireEvent.mouseDown(screen.getByTestId("outside"));

    await waitFor(() => {
      expect(screen.queryByRole("menu")).not.toBeInTheDocument();
    });
  });

  it("handles keyboard navigation", () => {
    render(
      <StatusQuickChange
        currentStatus="wishlist"
        onStatusChange={mockOnStatusChange}
      />
    );

    const button = screen.getByRole("button", { name: /change status/i });
    fireEvent.click(button);

    // Press Escape to close
    fireEvent.keyDown(button, { key: "Escape" });

    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
  });

  it("renders different sizes correctly", () => {
    const { rerender } = render(
      <StatusQuickChange
        currentStatus="wishlist"
        onStatusChange={mockOnStatusChange}
        size="sm"
      />
    );

    const button = screen.getByRole("button", { name: /change status/i });
    expect(button).toHaveClass("text-xs");

    rerender(
      <StatusQuickChange
        currentStatus="wishlist"
        onStatusChange={mockOnStatusChange}
        size="lg"
      />
    );

    expect(button).toHaveClass("text-base");
  });

  it("can hide label", () => {
    render(
      <StatusQuickChange
        currentStatus="wishlist"
        onStatusChange={mockOnStatusChange}
        showLabel={false}
      />
    );

    expect(screen.queryByText("Wishlist")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /change status/i })).toBeInTheDocument();
  });
});