import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { ActivityEntry, MessagePart } from "@/types/session";
import { ActivityItem, TaskToolActivity, ToolActivity } from "./ActivityLog";
import type { TaskGroup } from "./toolPairing";

describe("ToolActivity", () => {
	describe("ReadToolActivity", () => {
		it("shows collapsed read tool with label", () => {
			const entry = {
				type: "tool_use" as const,
				tool: "Read",
				input: { file_path: "/src/main.ts" },
				id: "t1",
			};
			render(<ToolActivity entry={entry} index={0} />);

			const el = screen.getByTestId("activity-tool-use-0");
			expect(el).toHaveTextContent("Explored /src/main.ts");
		});

		it("expands to show result content on click", () => {
			const entry = {
				type: "tool_use" as const,
				tool: "Read",
				input: { file_path: "/src/main.ts" },
				id: "t1",
			};
			const result = {
				type: "tool_result" as const,
				content: "file contents here",
				isError: false,
			};
			render(<ToolActivity entry={entry} result={result} index={0} />);

			expect(screen.queryByText("file contents here")).toBeNull();

			fireEvent.click(screen.getByText("Explored /src/main.ts"));
			expect(screen.getByText("file contents here")).toBeInTheDocument();
		});

		it("shows Glob tool with pattern label", () => {
			const entry = {
				type: "tool_use" as const,
				tool: "Glob",
				input: { pattern: "**/*.ts" },
				id: "t2",
			};
			render(<ToolActivity entry={entry} index={0} />);
			expect(screen.getByTestId("activity-tool-use-0")).toHaveTextContent(
				"Explored **/*.ts",
			);
		});
	});

	describe("basePath shortening", () => {
		it("shortens file_path label when basePath is provided", () => {
			const entry = {
				type: "tool_use" as const,
				tool: "Read",
				input: { file_path: "/home/user/project/src/main.ts" },
				id: "t-bp1",
			};
			render(
				<ToolActivity entry={entry} index={0} basePath="/home/user/project" />,
			);

			const el = screen.getByTestId("activity-tool-use-0");
			expect(el).toHaveTextContent("Explored src/main.ts");
		});

		it("shortens file_path in default tool when basePath is provided", () => {
			const entry = {
				type: "tool_use" as const,
				tool: "Edit",
				input: { file_path: "/home/user/project/src/app.ts" },
				id: "t-bp2",
			};
			render(
				<ToolActivity entry={entry} index={0} basePath="/home/user/project" />,
			);

			const el = screen.getByTestId("activity-tool-use-0");
			expect(el).toHaveTextContent("Edit src/app.ts");
		});
	});

	describe("truncate class", () => {
		it("applies truncate to read tool label", () => {
			const entry = {
				type: "tool_use" as const,
				tool: "Read",
				input: { file_path: "/src/main.ts" },
				id: "t-tr1",
			};
			render(<ToolActivity entry={entry} index={0} />);

			const el = screen.getByTestId("activity-tool-use-0");
			const span = el.querySelector("span.truncate");
			expect(span).not.toBeNull();
		});

		it("applies truncate to command tool label", () => {
			const entry = {
				type: "tool_use" as const,
				tool: "Bash",
				input: { command: "git status" },
				id: "t-tr2",
			};
			render(<ToolActivity entry={entry} index={0} />);

			const el = screen.getByTestId("activity-tool-use-0");
			const code = el.querySelector("code.truncate");
			expect(code).not.toBeNull();
		});

		it("applies truncate to default tool label", () => {
			const entry = {
				type: "tool_use" as const,
				tool: "Edit",
				input: { file_path: "/src/app.ts" },
				id: "t-tr3",
			};
			render(<ToolActivity entry={entry} index={0} />);

			const el = screen.getByTestId("activity-tool-use-0");
			const span = el.querySelector("span.truncate");
			expect(span).not.toBeNull();
		});
	});

	describe("CommandToolActivity", () => {
		it("shows command with terminal icon", () => {
			const entry = {
				type: "tool_use" as const,
				tool: "Bash",
				input: { command: "git status" },
				id: "t3",
			};
			render(<ToolActivity entry={entry} index={0} />);

			const el = screen.getByTestId("activity-tool-use-0");
			expect(el).toHaveTextContent("git status");
		});

		it("shows result as collapsible on label click", () => {
			const entry = {
				type: "tool_use" as const,
				tool: "Bash",
				input: { command: "ls" },
				id: "t4",
			};
			const result = {
				type: "tool_result" as const,
				content: "file1.ts\nfile2.ts",
				isError: false,
			};
			render(<ToolActivity entry={entry} result={result} index={0} />);

			expect(screen.queryByText(/file1\.ts/)).toBeNull();

			fireEvent.click(screen.getByText("ls"));
			expect(screen.getByText(/file1\.ts/)).toBeInTheDocument();
		});

		it("shows error result as collapsible", () => {
			const entry = {
				type: "tool_use" as const,
				tool: "Bash",
				input: { command: "bad-cmd" },
				id: "t5",
			};
			const result = {
				type: "tool_result" as const,
				content: "command not found",
				isError: true,
			};
			render(<ToolActivity entry={entry} result={result} index={0} />);

			expect(screen.getByText("Error")).toBeInTheDocument();
			expect(screen.queryByText("command not found")).toBeNull();

			fireEvent.click(screen.getByText("Error"));
			expect(screen.getByText("command not found")).toBeInTheDocument();
		});
	});

	describe("isExecuting spinner", () => {
		it("shows spinner when isExecuting is true for read tool", () => {
			const entry = {
				type: "tool_use" as const,
				tool: "Read",
				input: { file_path: "/src/main.ts" },
				id: "t1",
			};
			render(<ToolActivity entry={entry} index={0} isExecuting={true} />);
			const el = screen.getByTestId("activity-tool-use-0");
			expect(el.querySelector(".animate-spin")).not.toBeNull();
		});

		it("shows chevron when isExecuting is false for read tool", () => {
			const entry = {
				type: "tool_use" as const,
				tool: "Read",
				input: { file_path: "/src/main.ts" },
				id: "t1",
			};
			const result = {
				type: "tool_result" as const,
				content: "contents",
				isError: false,
			};
			render(
				<ToolActivity
					entry={entry}
					result={result}
					index={0}
					isExecuting={false}
				/>,
			);
			const el = screen.getByTestId("activity-tool-use-0");
			expect(el.querySelector(".animate-spin")).toBeNull();
		});

		it("shows spinner when isExecuting is true for command tool", () => {
			const entry = {
				type: "tool_use" as const,
				tool: "Bash",
				input: { command: "git status" },
				id: "t1",
			};
			render(<ToolActivity entry={entry} index={0} isExecuting={true} />);
			const el = screen.getByTestId("activity-tool-use-0");
			expect(el.querySelector(".animate-spin")).not.toBeNull();
		});

		it("shows spinner when isExecuting is true for default tool", () => {
			const entry = {
				type: "tool_use" as const,
				tool: "Edit",
				input: { file_path: "/src/app.ts" },
				id: "t1",
			};
			render(<ToolActivity entry={entry} index={0} isExecuting={true} />);
			const el = screen.getByTestId("activity-tool-use-0");
			expect(el.querySelector(".animate-spin")).not.toBeNull();
		});

		it("allows expand while executing", () => {
			const entry = {
				type: "tool_use" as const,
				tool: "Edit",
				input: { file_path: "/src/app.ts" },
				id: "t1",
			};
			render(<ToolActivity entry={entry} index={0} isExecuting={true} />);
			fireEvent.click(screen.getByText(/Edit/));
			expect(
				screen.getByText(/"file_path": "\/src\/app.ts"/),
			).toBeInTheDocument();
		});
	});

	describe("DefaultToolActivity", () => {
		it("shows write tool with name and summary", () => {
			const entry = {
				type: "tool_use" as const,
				tool: "Edit",
				input: { file_path: "/src/app.ts", old_string: "a", new_string: "b" },
				id: "t6",
			};
			render(<ToolActivity entry={entry} index={0} />);

			const el = screen.getByTestId("activity-tool-use-0");
			expect(el).toHaveTextContent("Edit");
			expect(el).toHaveTextContent("/src/app.ts");
		});

		it("toggles expand/collapse on label click", () => {
			const entry = {
				type: "tool_use" as const,
				tool: "Edit",
				input: { file_path: "/src/app.ts" },
				id: "t7",
			};
			render(<ToolActivity entry={entry} index={0} />);

			const label = screen.getByText("Edit /src/app.ts");
			fireEvent.click(label);
			expect(
				screen.getByText(/"file_path": "\/src\/app.ts"/),
			).toBeInTheDocument();

			fireEvent.click(label);
			expect(screen.queryByText(/"file_path": "\/src\/app.ts"/)).toBeNull();
		});
	});
});

describe("ActivityItem", () => {
	it("shows tool_result with error as collapsible", () => {
		const entry: ActivityEntry = {
			type: "tool_result",
			content: "permission denied",
			isError: true,
		};
		render(<ActivityItem entry={entry} index={1} />);

		const el = screen.getByTestId("activity-tool-result-1");
		expect(el).toHaveTextContent("Error");
		expect(screen.queryByText("permission denied")).toBeNull();

		fireEvent.click(screen.getByText("Error"));
		expect(screen.getByText("permission denied")).toBeInTheDocument();
	});

	it("shows tool_result without error", () => {
		const entry: ActivityEntry = {
			type: "tool_result",
			content: "file contents",
			isError: false,
		};
		render(<ActivityItem entry={entry} index={1} />);

		const el = screen.getByTestId("activity-tool-result-1");
		expect(el).toHaveTextContent("✓");
	});

	it("shows 'Done' for empty content result", () => {
		const entry: ActivityEntry = {
			type: "tool_result",
			content: "",
			isError: false,
		};
		render(<ActivityItem entry={entry} index={1} />);

		const el = screen.getByTestId("activity-tool-result-1");
		expect(el).toHaveTextContent("Done");
	});

	it("toggles expand/collapse for result content", () => {
		const entry: ActivityEntry = {
			type: "tool_result",
			content: "output text",
			isError: false,
		};
		render(<ActivityItem entry={entry} index={1} />);

		const label = screen.getByText("✓");
		fireEvent.click(label);

		expect(screen.getByText("output text")).toBeInTheDocument();

		fireEvent.click(label);
		expect(screen.queryByText("output text")).toBeNull();
	});

	it("truncates long result content", () => {
		const longContent = Array.from(
			{ length: 20 },
			(_, i) => `line ${i + 1}`,
		).join("\n");
		const entry: ActivityEntry = {
			type: "tool_result",
			content: longContent,
			isError: false,
		};
		render(<ActivityItem entry={entry} index={1} />);
		fireEvent.click(screen.getByText("✓"));

		expect(screen.getByText(/15 more lines/)).toBeInTheDocument();
	});

	it("shows tool_use fallback when no paired result", () => {
		const entry: ActivityEntry = {
			type: "tool_use",
			tool: "Read",
			input: { file_path: "/src/main.ts" },
			id: "t-orphan",
		};
		render(<ActivityItem entry={entry} index={2} />);
		expect(screen.getByTestId("activity-tool-use-2")).toBeDefined();
	});

	it("shows permission_result with allowed status", () => {
		const entry: ActivityEntry = {
			type: "permission_result",
			toolName: "Bash",
			status: "allowed",
			summary: "Bash: allowed",
		};
		render(<ActivityItem entry={entry} index={0} />);

		const el = screen.getByTestId("activity-permission-result-0");
		expect(el).toHaveTextContent("✓");
		expect(el).toHaveTextContent("Bash");
	});
});

describe("TaskToolActivity", () => {
	function makeTaskGroup(overrides: Partial<TaskGroup> = {}): TaskGroup {
		return {
			toolUseIndex: 0,
			toolUseId: "toolu_task_001",
			description: "Explore codebase",
			subagentType: "Explore",
			childIndices: [],
			statusParts: [],
			resultIndex: undefined,
			isCompleted: false,
			isBackground: false,
			completionStatusIndex: undefined,
			...overrides,
		};
	}

	const baseParts: MessagePart[] = [
		{
			type: "tool_use",
			tool: "Task",
			input: { description: "Explore codebase", subagent_type: "Explore" },
			id: "toolu_task_001",
		},
	];

	it("shows completed task in collapsed state with description and subagentType", () => {
		const group = makeTaskGroup({
			isCompleted: true,
			childIndices: [1, 2],
			statusParts: [
				{
					type: "task_status",
					taskToolUseId: "toolu_task_001",
					status: "completed",
					summary: "Done",
				},
			],
		});
		const parts: MessagePart[] = [
			...baseParts,
			{
				type: "tool_use",
				tool: "Read",
				input: { file_path: "/src/main.ts" },
				id: "toolu_child_001",
				parentToolUseId: "toolu_task_001",
			},
			{
				type: "tool_result",
				content: "file content",
				isError: false,
				toolUseId: "toolu_child_001",
				parentToolUseId: "toolu_task_001",
			},
		];
		render(
			<TaskToolActivity
				group={group}
				parts={parts}
				pairedResults={new Map()}
				isStreaming={false}
			/>,
		);

		const el = screen.getByTestId("activity-task-0");
		expect(el).toHaveTextContent("Explore codebase");
		// Children should not be visible (collapsed)
		expect(screen.queryByTestId("activity-tool-use-1")).toBeNull();
	});

	it("shows running task in collapsed state by default", () => {
		const group = makeTaskGroup({
			childIndices: [1],
		});
		const parts: MessagePart[] = [
			...baseParts,
			{
				type: "tool_use",
				tool: "Read",
				input: { file_path: "/src/main.ts" },
				id: "toolu_child_001",
				parentToolUseId: "toolu_task_001",
			},
		];
		render(
			<TaskToolActivity
				group={group}
				parts={parts}
				pairedResults={new Map()}
				isStreaming={true}
			/>,
		);

		const el = screen.getByTestId("activity-task-0");
		// Running task shows spinner
		expect(el.querySelector(".animate-spin")).not.toBeNull();
		// Children not visible (collapsed by default)
		expect(screen.queryByTestId("activity-tool-use-1")).toBeNull();
	});

	it("shows spinner for background task even when not streaming", () => {
		const group = makeTaskGroup({
			isBackground: true,
			isCompleted: false,
			childIndices: [1],
		});
		const parts: MessagePart[] = [
			...baseParts,
			{
				type: "tool_use",
				tool: "Read",
				input: { file_path: "/src/main.ts" },
				id: "toolu_child_001",
				parentToolUseId: "toolu_task_001",
			},
		];
		render(
			<TaskToolActivity
				group={group}
				parts={parts}
				pairedResults={new Map()}
				isStreaming={false}
			/>,
		);

		const el = screen.getByTestId("activity-task-0");
		expect(el.querySelector(".animate-spin")).not.toBeNull();
	});

	it("hides spinner for completed background task", () => {
		const group = makeTaskGroup({
			isBackground: true,
			isCompleted: true,
			childIndices: [1],
			statusParts: [
				{
					type: "task_status",
					taskToolUseId: "toolu_task_001",
					status: "completed",
					summary: "Done",
				},
			],
		});
		const parts: MessagePart[] = [
			...baseParts,
			{
				type: "tool_use",
				tool: "Read",
				input: { file_path: "/src/main.ts" },
				id: "toolu_child_001",
				parentToolUseId: "toolu_task_001",
			},
		];
		render(
			<TaskToolActivity
				group={group}
				parts={parts}
				pairedResults={new Map()}
				isStreaming={false}
			/>,
		);

		const el = screen.getByTestId("activity-task-0");
		expect(el.querySelector(".animate-spin")).toBeNull();
	});

	it("toggles expand/collapse on label click", () => {
		const group = makeTaskGroup({
			isCompleted: true,
			childIndices: [1],
			statusParts: [
				{
					type: "task_status",
					taskToolUseId: "toolu_task_001",
					status: "completed",
				},
			],
		});
		const parts: MessagePart[] = [
			...baseParts,
			{
				type: "tool_use",
				tool: "Read",
				input: { file_path: "/src/main.ts" },
				id: "toolu_child_001",
				parentToolUseId: "toolu_task_001",
			},
		];
		render(
			<TaskToolActivity
				group={group}
				parts={parts}
				pairedResults={new Map()}
				isStreaming={false}
			/>,
		);

		// Completed → initially collapsed
		expect(screen.queryByTestId("activity-tool-use-1")).toBeNull();

		// Click to expand
		fireEvent.click(screen.getByTestId("activity-task-0"));
		expect(screen.getByTestId("activity-tool-use-1")).toBeDefined();

		// Click to collapse
		fireEvent.click(screen.getByTestId("activity-task-0"));
		expect(screen.queryByTestId("activity-tool-use-1")).toBeNull();
	});

	it("shows child tool_use entries when expanded by click", () => {
		const childResult: Extract<MessagePart, { type: "tool_result" }> = {
			type: "tool_result",
			content: "file content here",
			isError: false,
			toolUseId: "toolu_child_001",
		};
		const group = makeTaskGroup({
			childIndices: [1],
		});
		const parts: MessagePart[] = [
			...baseParts,
			{
				type: "tool_use",
				tool: "Read",
				input: { file_path: "/src/main.ts" },
				id: "toolu_child_001",
				parentToolUseId: "toolu_task_001",
			},
		];
		const pairedResults = new Map<
			number,
			Extract<MessagePart, { type: "tool_result" }>
		>([[1, childResult]]);

		render(
			<TaskToolActivity
				group={group}
				parts={parts}
				pairedResults={pairedResults}
				isStreaming={false}
			/>,
		);

		// Collapsed by default
		expect(screen.queryByTestId("activity-tool-use-1")).toBeNull();

		// Click to expand
		fireEvent.click(screen.getByTestId("activity-task-0"));
		const toolEl = screen.getByTestId("activity-tool-use-1");
		expect(toolEl).toHaveTextContent("Explored /src/main.ts");
	});

	it("shows description with subagentType suffix when both present", () => {
		const group = makeTaskGroup({
			isCompleted: true,
			childIndices: [],
			statusParts: [
				{
					type: "task_status",
					taskToolUseId: "toolu_task_001",
					status: "completed",
				},
			],
		});
		render(
			<TaskToolActivity
				group={group}
				parts={baseParts}
				pairedResults={new Map()}
				isStreaming={false}
			/>,
		);

		const el = screen.getByTestId("activity-task-0");
		expect(el).toHaveTextContent("Explore codebase (Explore)");
	});

	it("shows sub-agent text output when expanded by click", () => {
		const group = makeTaskGroup({
			childIndices: [1],
		});
		const parts: MessagePart[] = [
			...baseParts,
			{
				type: "text",
				content: "Analysis result: found 3 components",
				parentToolUseId: "toolu_task_001",
			},
		];
		render(
			<TaskToolActivity
				group={group}
				parts={parts}
				pairedResults={new Map()}
				isStreaming={false}
			/>,
		);

		// Collapsed by default
		expect(
			screen.queryByText("Analysis result: found 3 components"),
		).toBeNull();

		// Click to expand
		fireEvent.click(screen.getByTestId("activity-task-0"));
		expect(
			screen.getByText("Analysis result: found 3 components"),
		).toBeInTheDocument();
	});

	it("shows fallback label with subagentType when no description", () => {
		const group = makeTaskGroup({
			description: undefined,
			subagentType: "Explore",
		});
		render(
			<TaskToolActivity
				group={group}
				parts={baseParts}
				pairedResults={new Map()}
				isStreaming={false}
			/>,
		);

		expect(screen.getByTestId("activity-task-0")).toHaveTextContent(
			"Task (Explore)",
		);
	});
});
