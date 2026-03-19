import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { SessionSummary } from "@/types/session";
import { SessionList } from "./SessionList";

function makeSummary(
	id: string,
	firstMessage: string,
	messageCount: number,
): SessionSummary {
	return {
		id,
		worktreePath: "/repo",
		state: "idle",
		createdAt: 1000,
		updatedAt: 1000,
		firstMessage,
		messageCount,
	};
}

describe("SessionList", () => {
	it("renders session items", () => {
		const sessions = [
			makeSummary("s1", "Hello", 3),
			makeSummary("s2", "Fix bug", 5),
		];
		render(
			<SessionList
				sessions={sessions}
				activeSessionId={null}
				onSelect={vi.fn()}
				onNew={vi.fn()}
			/>,
		);
		expect(screen.getByText("Hello")).toBeDefined();
		expect(screen.getByText("Fix bug")).toBeDefined();
		expect(screen.getByText("3 messages")).toBeDefined();
		expect(screen.getByText("5 messages")).toBeDefined();
	});

	it("shows empty state when no sessions", () => {
		render(
			<SessionList
				sessions={[]}
				activeSessionId={null}
				onSelect={vi.fn()}
				onNew={vi.fn()}
			/>,
		);
		expect(screen.getByText("No sessions yet")).toBeDefined();
	});

	it("highlights active session", () => {
		const sessions = [
			makeSummary("s1", "Hello", 3),
			makeSummary("s2", "Fix bug", 5),
		];
		const { container } = render(
			<SessionList
				sessions={sessions}
				activeSessionId="s1"
				onSelect={vi.fn()}
				onNew={vi.fn()}
			/>,
		);
		const buttons = container.querySelectorAll("button[type='button']");
		const activeButton = Array.from(buttons).find((b) =>
			b.className.includes("bg-muted"),
		);
		expect(activeButton).toBeDefined();
		expect(activeButton?.textContent).toContain("Hello");
	});

	it("calls onSelect when session is clicked", () => {
		const onSelect = vi.fn();
		const sessions = [makeSummary("s1", "Hello", 3)];
		render(
			<SessionList
				sessions={sessions}
				activeSessionId={null}
				onSelect={onSelect}
				onNew={vi.fn()}
			/>,
		);
		fireEvent.click(screen.getByText("Hello"));
		expect(onSelect).toHaveBeenCalledWith("s1");
	});

	it("calls onNew when new session button is clicked", () => {
		const onNew = vi.fn();
		render(
			<SessionList
				sessions={[]}
				activeSessionId={null}
				onSelect={vi.fn()}
				onNew={onNew}
			/>,
		);
		fireEvent.click(screen.getByLabelText("New session"));
		expect(onNew).toHaveBeenCalled();
	});

	it("shows 'New session' for sessions without first message", () => {
		const sessions = [makeSummary("s1", "", 0)];
		render(
			<SessionList
				sessions={sessions}
				activeSessionId={null}
				onSelect={vi.fn()}
				onNew={vi.fn()}
			/>,
		);
		expect(screen.getByText("New session")).toBeDefined();
	});
});
