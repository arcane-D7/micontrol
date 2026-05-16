import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import ChargingThreshold from "../components/ChargingThreshold";

describe("ChargingThreshold", () => {
  const mockOnChange = vi.fn().mockResolvedValue(undefined);

  beforeEach(() => {
    mockOnChange.mockClear();
  });

  it("renders all threshold options", () => {
    render(<ChargingThreshold threshold={80} onThresholdChange={mockOnChange} />);
    expect(screen.getByText("40%")).toBeInTheDocument();
    expect(screen.getByText("50%")).toBeInTheDocument();
    expect(screen.getByText("60%")).toBeInTheDocument();
    expect(screen.getByText("70%")).toBeInTheDocument();
    expect(screen.getByText("80%")).toBeInTheDocument();
  });

  it("marks the current threshold as active", () => {
    render(<ChargingThreshold threshold={70} onThresholdChange={mockOnChange} />);
    const btn70 = screen.getByText("70%").closest("button");
    expect(btn70).toHaveClass("active");
  });

  it("calls onThresholdChange when a threshold is selected", () => {
    render(<ChargingThreshold threshold={80} onThresholdChange={mockOnChange} />);
    fireEvent.click(screen.getByText("60%").closest("button")!);
    expect(mockOnChange).toHaveBeenCalledWith(60);
  });

  it("shows the recommended badge on 80%", () => {
    render(<ChargingThreshold threshold={80} onThresholdChange={mockOnChange} />);
    expect(screen.getByText("Recommended")).toBeInTheDocument();
  });

  it("renders section title", () => {
    render(<ChargingThreshold threshold={80} onThresholdChange={mockOnChange} />);
    expect(screen.getByText("Charging Control")).toBeInTheDocument();
  });
});
