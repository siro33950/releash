import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { ActivityEntry } from "@/types/session";
import { ActivityItem, ToolActivity } from "./ActivityLog";

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
