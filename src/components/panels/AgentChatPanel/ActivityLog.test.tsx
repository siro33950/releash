import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { ActivityEntry } from "@/types/session";
import { ActivityLog } from "./ActivityLog";

describe("ActivityLog", () => {
	it("returns null when activities is empty", () => {
		const { container } = render(
			<ActivityLog activities={[]} isStreaming={false} />,
		);
		expect(container.firstChild).toBeNull();
	});

	it("shows tool use count", () => {
		const activities: ActivityEntry[] = [
			{
				type: "tool_use",
				tool: "Read",
				input: { file_path: "/a.ts" },
				id: "t1",
			},
			{ type: "tool_result", content: "ok", isError: false },
		];
		render(<ActivityLog activities={activities} isStreaming={false} />);
		expect(screen.getByTestId("activity-log-toggle")).toHaveTextContent(
			"1 tool call",
		);
	});

	it("shows plural tool calls count", () => {
		const activities: ActivityEntry[] = [
			{ type: "tool_use", tool: "Read", input: {}, id: "t1" },
			{ type: "tool_result", content: "", isError: false },
			{ type: "tool_use", tool: "Grep", input: { pattern: "TODO" }, id: "t2" },
			{ type: "tool_result", content: "found", isError: false },
		];
		render(<ActivityLog activities={activities} isStreaming={false} />);
		expect(screen.getByTestId("activity-log-toggle")).toHaveTextContent(
			"2 tool calls",
		);
	});

	it("shows spinner when streaming", () => {
		const activities: ActivityEntry[] = [
			{ type: "tool_use", tool: "Read", input: {}, id: "t1" },
		];
		const { container } = render(
			<ActivityLog activities={activities} isStreaming={true} />,
		);
		expect(container.querySelector(".animate-spin")).toBeTruthy();
	});

	it("does not show spinner when not streaming", () => {
		const activities: ActivityEntry[] = [
			{ type: "tool_use", tool: "Read", input: {}, id: "t1" },
		];
		const { container } = render(
			<ActivityLog activities={activities} isStreaming={false} />,
		);
		expect(container.querySelector(".animate-spin")).toBeFalsy();
	});

	it("expands to show activity items on toggle click", () => {
		const activities: ActivityEntry[] = [
			{
				type: "tool_use",
				tool: "Read",
				input: { file_path: "/src/main.ts" },
				id: "t1",
			},
		];
		render(<ActivityLog activities={activities} isStreaming={false} />);

		expect(screen.queryByTestId("activity-tool-use-0")).toBeNull();

		fireEvent.click(screen.getByTestId("activity-log-toggle"));

		expect(screen.getByTestId("activity-tool-use-0")).toBeInTheDocument();
		expect(screen.getByTestId("activity-tool-use-0")).toHaveTextContent("Read");
		expect(screen.getByTestId("activity-tool-use-0")).toHaveTextContent(
			"/src/main.ts",
		);
	});

	it("shows tool_result with error styling", () => {
		const activities: ActivityEntry[] = [
			{ type: "tool_use", tool: "Bash", input: { command: "ls" }, id: "t1" },
			{ type: "tool_result", content: "permission denied", isError: true },
		];
		render(<ActivityLog activities={activities} isStreaming={false} />);
		fireEvent.click(screen.getByTestId("activity-log-toggle"));

		const resultEl = screen.getByTestId("activity-tool-result-1");
		expect(resultEl).toHaveTextContent("Error");
	});

	it("shows tool_result without error", () => {
		const activities: ActivityEntry[] = [
			{ type: "tool_use", tool: "Read", input: {}, id: "t1" },
			{ type: "tool_result", content: "file contents", isError: false },
		];
		render(<ActivityLog activities={activities} isStreaming={false} />);
		fireEvent.click(screen.getByTestId("activity-log-toggle"));

		const resultEl = screen.getByTestId("activity-tool-result-1");
		expect(resultEl).toHaveTextContent("✓");
	});

	it("shows 'Done' for empty content result", () => {
		const activities: ActivityEntry[] = [
			{ type: "tool_use", tool: "Write", input: {}, id: "t1" },
			{ type: "tool_result", content: "", isError: false },
		];
		render(<ActivityLog activities={activities} isStreaming={false} />);
		fireEvent.click(screen.getByTestId("activity-log-toggle"));

		const resultEl = screen.getByTestId("activity-tool-result-1");
		expect(resultEl).toHaveTextContent("Done");
	});

	it("toggles show/hide for result content", () => {
		const activities: ActivityEntry[] = [
			{ type: "tool_use", tool: "Bash", input: {}, id: "t1" },
			{ type: "tool_result", content: "output text", isError: false },
		];
		render(<ActivityLog activities={activities} isStreaming={false} />);
		fireEvent.click(screen.getByTestId("activity-log-toggle"));

		const showBtn = screen.getByText("show");
		fireEvent.click(showBtn);

		expect(screen.getByText("output text")).toBeInTheDocument();
		expect(screen.getByText("hide")).toBeInTheDocument();

		fireEvent.click(screen.getByText("hide"));
		expect(screen.queryByText("output text")).toBeNull();
	});

	it("truncates long result content", () => {
		const longContent = Array.from(
			{ length: 20 },
			(_, i) => `line ${i + 1}`,
		).join("\n");
		const activities: ActivityEntry[] = [
			{ type: "tool_use", tool: "Bash", input: {}, id: "t1" },
			{ type: "tool_result", content: longContent, isError: false },
		];
		render(<ActivityLog activities={activities} isStreaming={false} />);
		fireEvent.click(screen.getByTestId("activity-log-toggle"));
		fireEvent.click(screen.getByText("show"));

		expect(screen.getByText(/15 more lines/)).toBeInTheDocument();
	});
});
