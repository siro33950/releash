import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { ReviewThread } from "@/types/protocol";
import { RemoteReviewPanel } from "./RemoteReviewPanel";

const makeThread = (overrides: Partial<ReviewThread> = {}): ReviewThread => ({
	id: "t1",
	worktreeName: "/repo",
	author: { kind: "agent", displayName: "codex/gpt-5" },
	target: {},
	state: "open",
	comments: [
		{
			id: "c1",
			threadId: "t1",
			author: { kind: "agent", displayName: "codex/gpt-5" },
			content: "Initial claim",
			createdAt: 1,
		},
		{
			id: "c2",
			threadId: "t1",
			author: { kind: "human", displayName: "Human" },
			content: "Reply text",
			createdAt: 2,
		},
	],
	resolve: null,
	createdAt: 1,
	updatedAt: 2,
	version: 2,
	canResolve: true,
	...overrides,
});

function renderPanel(
	overrides: Partial<Parameters<typeof RemoteReviewPanel>[0]> = {},
) {
	const thread = makeThread();
	const props = {
		threads: [thread],
		selectedThread: thread,
		selectedThreadId: thread.id,
		loading: false,
		error: null,
		onSelectThread: vi.fn(),
		onRefresh: vi.fn(),
		onCreateThread: vi.fn(),
		onAppendComment: vi.fn(),
		onResolveThread: vi.fn(),
		...overrides,
	};
	render(<RemoteReviewPanel {...props} />);
	return props;
}

describe("RemoteReviewPanel", () => {
	it("renders comments, resolve metadata, and callbacks", () => {
		const props = renderPanel({
			selectedThread: makeThread({
				state: "resolved",
				canResolve: false,
				resolve: {
					actor: { kind: "human", displayName: "Human" },
					outcome: "resolved",
					summary: "Fixed",
					resolvedAt: 3,
				},
			}),
		});

		expect(screen.getAllByText("Initial claim").length).toBeGreaterThan(0);
		expect(screen.getByText("Reply text")).toBeInTheDocument();
		expect(screen.getByText("Fixed")).toBeInTheDocument();

		fireEvent.click(screen.getAllByText("Initial claim")[0]);
		expect(props.onSelectThread).toHaveBeenCalledWith("t1");
		expect(screen.queryByPlaceholderText("Reply...")).not.toBeInTheDocument();
	});

	it("disables create while a create request is pending", () => {
		const props = renderPanel();

		fireEvent.change(screen.getByPlaceholderText("Start a thread..."), {
			target: { value: "New thread" },
		});
		const create = screen.getByText("Create");
		fireEvent.click(create);
		fireEvent.click(create);
		expect(props.onCreateThread).toHaveBeenCalledTimes(1);
		expect(create).toBeDisabled();
	});

	it("disables reply while a reply request is pending", () => {
		const props = renderPanel();

		fireEvent.change(screen.getByPlaceholderText("Reply..."), {
			target: { value: "A reply" },
		});
		const reply = screen.getByText("Reply");
		fireEvent.click(reply);
		fireEvent.click(reply);

		expect(props.onAppendComment).toHaveBeenCalledTimes(1);
		expect(reply).toBeDisabled();
	});

	it("disables resolve while a resolve request is pending", () => {
		const props = renderPanel();

		fireEvent.change(screen.getByPlaceholderText("Resolution summary"), {
			target: { value: "Done" },
		});
		const resolve = screen.getByText("Resolve");
		fireEvent.click(resolve);
		fireEvent.click(resolve);

		expect(props.onResolveThread).toHaveBeenCalledTimes(1);
		expect(resolve).toBeDisabled();
	});

	it("appends a comment via Reply form", () => {
		const props = renderPanel();

		fireEvent.change(screen.getByPlaceholderText("Reply..."), {
			target: { value: "Just a reply" },
		});
		fireEvent.click(screen.getByText("Reply"));

		expect(props.onAppendComment).toHaveBeenCalledWith("t1", "Just a reply");
	});

	it("does not show a Reply form for resolved threads", () => {
		renderPanel({
			selectedThread: makeThread({
				state: "resolved",
				canResolve: false,
			}),
		});

		expect(screen.queryByPlaceholderText("Reply...")).not.toBeInTheDocument();
	});
});
