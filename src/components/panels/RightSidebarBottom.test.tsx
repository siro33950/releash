import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { Thread } from "@/types/thread";
import { RightSidebarBottom } from "./RightSidebarBottom";

// react-resizable-panels does not work in jsdom
vi.mock("react-resizable-panels", () => ({
	Group: ({ children }: { children: React.ReactNode }) => (
		<div data-testid="resizable-group">{children}</div>
	),
	Panel: ({
		children,
		id,
	}: {
		children: React.ReactNode;
		id?: string;
		[key: string]: unknown;
	}) => <div data-testid={`panel-${id ?? "unknown"}`}>{children}</div>,
	Separator: () => <div data-testid="separator" />,
}));

vi.mock("@/components/panels/CommentList", () => ({
	CommentList: (props: {
		threads: Thread[];
		showResolvedThreads?: boolean;
		onToggleShowResolved?: () => void;
	}) => (
		<div data-testid="comment-list">
			{props.threads.length} threads
			{props.onToggleShowResolved && (
				<button type="button" onClick={props.onToggleShowResolved}>
					Toggle resolved
				</button>
			)}
		</div>
	),
}));

vi.mock("@/components/panels/TerminalPanel", () => ({
	TerminalPanel: () => <div data-testid="terminal-panel" />,
}));

vi.mock("@/lib/formatCommentsForTerminal", () => ({
	formatCommentsForTerminal: (threads: Thread[]) =>
		threads.map((t) => t.entries[0]?.content).join("\n"),
}));

function makeThread(overrides: Partial<Thread> = {}): Thread {
	return {
		id: `thread-${Math.random().toString(36).slice(2)}`,
		filePath: "src/index.ts",
		lineNumber: 1,
		entries: [{ id: "e1", content: "Fix this", createdAt: 1 }],
		resolved: false,
		createdAt: Date.now(),
		...overrides,
	};
}

const defaultProps = {
	rootPath: "/repo",
	threads: [] as Thread[],
};

describe("RightSidebarBottom", () => {
	it("should render terminal and comments panels side by side", () => {
		render(<RightSidebarBottom {...defaultProps} />);

		expect(screen.getByTestId("panel-terminal")).toBeInTheDocument();
		expect(screen.getByTestId("panel-comments")).toBeInTheDocument();
		expect(screen.getByTestId("resizable-group")).toBeInTheDocument();
	});

	it("should render a separator between panels", () => {
		render(<RightSidebarBottom {...defaultProps} />);

		expect(screen.getByTestId("separator")).toBeInTheDocument();
	});

	it("should render collapse button with correct aria-label when expanded", () => {
		render(
			<RightSidebarBottom
				{...defaultProps}
				collapsed={false}
				onToggleCollapse={vi.fn()}
			/>,
		);

		expect(
			screen.getByRole("button", { name: "Collapse panel" }),
		).toBeInTheDocument();
	});

	it("should render expand button with correct aria-label when collapsed", () => {
		render(
			<RightSidebarBottom
				{...defaultProps}
				collapsed={true}
				onToggleCollapse={vi.fn()}
			/>,
		);

		expect(
			screen.getByRole("button", { name: "Expand panel" }),
		).toBeInTheDocument();
	});

	it("should call onToggleCollapse when collapse button is clicked", async () => {
		const user = userEvent.setup();
		const onToggleCollapse = vi.fn();

		render(
			<RightSidebarBottom
				{...defaultProps}
				collapsed={false}
				onToggleCollapse={onToggleCollapse}
			/>,
		);

		await user.click(screen.getByRole("button", { name: "Collapse panel" }));
		expect(onToggleCollapse).toHaveBeenCalledOnce();
	});

	it("should copy unresolved comments to clipboard when copy button is clicked", async () => {
		const user = userEvent.setup();
		const writeText = vi.fn().mockResolvedValue(undefined);
		Object.defineProperty(navigator, "clipboard", {
			value: { writeText },
			writable: true,
			configurable: true,
		});

		const threads = [
			makeThread({ resolved: false }),
			makeThread({ resolved: true }),
		];

		render(<RightSidebarBottom {...defaultProps} threads={threads} />);

		await user.click(
			screen.getByRole("button", { name: "Copy comments to clipboard" }),
		);
		expect(writeText).toHaveBeenCalledOnce();
		expect(writeText).toHaveBeenCalledWith("Fix this");
	});

	it("should call onSendToTerminal with unresolved threads when send button is clicked", async () => {
		const user = userEvent.setup();
		const onSendToTerminal = vi.fn();

		const unresolvedThread = makeThread({ resolved: false });
		const resolvedThread = makeThread({ resolved: true });

		render(
			<RightSidebarBottom
				{...defaultProps}
				threads={[unresolvedThread, resolvedThread]}
				onSendToTerminal={onSendToTerminal}
			/>,
		);

		await user.click(screen.getByText("Send"));
		expect(onSendToTerminal).toHaveBeenCalledOnce();
		expect(onSendToTerminal).toHaveBeenCalledWith([unresolvedThread]);
	});

	it("should disable copy and send buttons when no unresolved threads exist", () => {
		const threads = [makeThread({ resolved: true })];

		render(
			<RightSidebarBottom
				{...defaultProps}
				threads={threads}
				onSendToTerminal={vi.fn()}
			/>,
		);

		expect(
			screen.getByRole("button", { name: "Copy comments to clipboard" }),
		).toBeDisabled();
		expect(screen.getByText("Send").closest("button")).toBeDisabled();
	});

	it("should not render send button when onSendToTerminal is not provided", () => {
		render(<RightSidebarBottom {...defaultProps} />);

		expect(screen.queryByText("Send")).not.toBeInTheDocument();
	});

	it("should not render collapse button when onToggleCollapse is not provided", () => {
		render(<RightSidebarBottom {...defaultProps} />);

		expect(
			screen.queryByRole("button", { name: "Collapse panel" }),
		).not.toBeInTheDocument();
		expect(
			screen.queryByRole("button", { name: "Expand panel" }),
		).not.toBeInTheDocument();
	});
});
