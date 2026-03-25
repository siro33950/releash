import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { ShimmerPlaceholder } from "./ShimmerPlaceholder";

describe("ShimmerPlaceholder", () => {
	it("renders 3 lines by default", () => {
		const { container } = render(<ShimmerPlaceholder />);
		const bars = container.querySelectorAll(".agent-shimmer");
		expect(bars.length).toBe(3);
	});

	it("renders specified number of lines", () => {
		const { container } = render(<ShimmerPlaceholder lines={5} />);
		const bars = container.querySelectorAll(".agent-shimmer");
		expect(bars.length).toBe(5);
	});

	it("last line has reduced width", () => {
		const { container } = render(<ShimmerPlaceholder lines={3} />);
		const bars = container.querySelectorAll(".agent-shimmer");
		const lastBar = bars[bars.length - 1];
		expect(lastBar.className).toContain("w-3/5");
	});

	it("non-last lines have full width", () => {
		const { container } = render(<ShimmerPlaceholder lines={3} />);
		const bars = container.querySelectorAll(".agent-shimmer");
		expect(bars[0].className).toContain("w-full");
		expect(bars[1].className).toContain("w-full");
	});

	it("has testid", () => {
		render(<ShimmerPlaceholder />);
		expect(screen.getByTestId("shimmer-placeholder")).toBeDefined();
	});
});
