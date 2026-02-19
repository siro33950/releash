import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { AgentStateBadge, formatElapsed } from "./agent-state-badge";

const states = ["running", "done", "waiting", "error"] as const;

describe("formatElapsed", () => {
	it("should return seconds when diff < 60", () => {
		const now = Date.now() / 1000;
		expect(formatElapsed(now - 30)).toBe("30s");
	});

	it("should return minutes when diff >= 60 and < 3600", () => {
		const now = Date.now() / 1000;
		expect(formatElapsed(now - 120)).toBe("2m");
	});

	it("should return hours when diff >= 3600", () => {
		const now = Date.now() / 1000;
		expect(formatElapsed(now - 7200)).toBe("2h");
	});

	it("should clamp negative diff to 0", () => {
		const now = Date.now() / 1000;
		expect(formatElapsed(now + 100)).toBe("0s");
	});
});

describe("AgentStateBadge", () => {
	describe("badge variant (default)", () => {
		for (const state of states) {
			it(`should render ${state} state`, () => {
				render(<AgentStateBadge state={state} />);
				const label = state.charAt(0).toUpperCase() + state.slice(1);
				expect(screen.getByText(label)).toBeInTheDocument();
			});
		}

		it("should show elapsed time when timestamp is provided", () => {
			const now = Date.now() / 1000;
			render(<AgentStateBadge state="running" timestamp={now - 30} />);
			expect(screen.getByText("30s")).toBeInTheDocument();
		});

		it("should not show elapsed time when timestamp is not provided", () => {
			render(<AgentStateBadge state="running" />);
			expect(screen.queryByText(/\d+[smh]/)).not.toBeInTheDocument();
		});

		it("should have animate-pulse for running state", () => {
			const { container } = render(<AgentStateBadge state="running" />);
			const dot = container.querySelector(".rounded-full");
			expect(dot?.className).toContain("animate-pulse");
		});

		it("should have animate-pulse for waiting state", () => {
			const { container } = render(<AgentStateBadge state="waiting" />);
			const dot = container.querySelector(".rounded-full");
			expect(dot?.className).toContain("animate-pulse");
		});

		it("should not have animate-pulse for done state", () => {
			const { container } = render(<AgentStateBadge state="done" />);
			const dot = container.querySelector(".rounded-full");
			expect(dot?.className).not.toContain("animate-pulse");
		});

		it("should not have animate-pulse for error state", () => {
			const { container } = render(<AgentStateBadge state="error" />);
			const dot = container.querySelector(".rounded-full");
			expect(dot?.className).not.toContain("animate-pulse");
		});
	});

	describe("dot variant", () => {
		for (const state of states) {
			it(`should render ${state} state as dot with title`, () => {
				render(<AgentStateBadge state={state} variant="dot" />);
				expect(screen.getByTitle(state)).toBeInTheDocument();
			});
		}

		it("should have w-2 h-2 classes", () => {
			render(<AgentStateBadge state="running" variant="dot" />);
			const dot = screen.getByTitle("running");
			expect(dot.className).toContain("w-2");
			expect(dot.className).toContain("h-2");
		});

		it("should have animate-pulse for running", () => {
			render(<AgentStateBadge state="running" variant="dot" />);
			expect(screen.getByTitle("running").className).toContain("animate-pulse");
		});

		it("should not have animate-pulse for done", () => {
			render(<AgentStateBadge state="done" variant="dot" />);
			expect(screen.getByTitle("done").className).not.toContain(
				"animate-pulse",
			);
		});
	});

	describe("inline variant", () => {
		for (const state of states) {
			it(`should render ${state} state with Agent: prefix`, () => {
				render(<AgentStateBadge state={state} variant="inline" />);
				expect(screen.getByText(`Agent: ${state}`)).toBeInTheDocument();
			});
		}

		it("should have animate-pulse for running", () => {
			const { container } = render(
				<AgentStateBadge state="running" variant="inline" />,
			);
			const dot = container.querySelector(".rounded-full");
			expect(dot?.className).toContain("animate-pulse");
		});

		it("should not have animate-pulse for done", () => {
			const { container } = render(
				<AgentStateBadge state="done" variant="inline" />,
			);
			const dot = container.querySelector(".rounded-full");
			expect(dot?.className).not.toContain("animate-pulse");
		});
	});
});
