import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { Thread } from "@/types/thread";
import { CommentList } from "./CommentList";

function makeThread(overrides: Partial<Thread> = {}): Thread {
	return {
		id: "t-1",
		filePath: "/src/App.tsx",
		lineNumber: 10,
		entries: [
			{
				id: "e-1",
				content: "test comment",
				createdAt: Date.now(),
			},
		],
		createdAt: Date.now(),
		resolved: false,
		...overrides,
	};
}

describe("CommentList", () => {
	it("should show empty state with hints when no comments", () => {
		render(<CommentList threads={[]} />);
		expect(screen.getByText("No comments")).toBeInTheDocument();
		expect(
			screen.getByText(/Click the left margin of a line number/),
		).toBeInTheDocument();
		expect(screen.getByText("⌘K")).toBeInTheDocument();
	});

	it("should display file name and comment content", () => {
		render(
			<CommentList
				threads={[
					makeThread({
						entries: [
							{
								id: "e-1",
								content: "fix this bug",
								createdAt: Date.now(),
							},
						],
					}),
				]}
			/>,
		);
		expect(screen.getByText("App.tsx")).toBeInTheDocument();
		expect(screen.getByText("fix this bug")).toBeInTheDocument();
	});

	it("should display line number", () => {
		render(<CommentList threads={[makeThread({ lineNumber: 42 })]} />);
		expect(screen.getByText("L42")).toBeInTheDocument();
	});

	it("should display range line number with endLine", () => {
		render(
			<CommentList threads={[makeThread({ lineNumber: 5, endLine: 12 })]} />,
		);
		expect(screen.getByText("L5-12")).toBeInTheDocument();
	});

	it("should call onThreadClick when comment is clicked", async () => {
		const user = userEvent.setup();
		const onClick = vi.fn();
		render(
			<CommentList
				threads={[
					makeThread({
						lineNumber: 15,
						entries: [
							{
								id: "e-1",
								content: "click me",
								createdAt: Date.now(),
							},
						],
					}),
				]}
				onThreadClick={onClick}
			/>,
		);
		await user.click(screen.getByText("click me"));
		expect(onClick).toHaveBeenCalledWith("/src/App.tsx", 15);
	});

	it("should hide resolved threads when showResolvedThreads is false", () => {
		render(
			<CommentList
				threads={[
					makeThread({
						id: "t-1",
						entries: [
							{
								id: "e-1",
								content: "active one",
								createdAt: Date.now(),
							},
						],
					}),
					makeThread({
						id: "t-2",
						resolved: true,
						lineNumber: 20,
						entries: [
							{
								id: "e-2",
								content: "resolved one",
								createdAt: Date.now(),
							},
						],
					}),
				]}
				showResolvedThreads={false}
				onToggleShowResolved={vi.fn()}
			/>,
		);
		expect(screen.getByText("active one")).toBeInTheDocument();
		expect(screen.queryByText("resolved one")).not.toBeInTheDocument();
	});

	it("should show resolved threads when showResolvedThreads is true", () => {
		render(
			<CommentList
				threads={[
					makeThread({
						id: "t-1",
						entries: [
							{
								id: "e-1",
								content: "active one",
								createdAt: Date.now(),
							},
						],
					}),
					makeThread({
						id: "t-2",
						resolved: true,
						lineNumber: 20,
						entries: [
							{
								id: "e-2",
								content: "resolved one",
								createdAt: Date.now(),
							},
						],
					}),
				]}
				showResolvedThreads={true}
				onToggleShowResolved={vi.fn()}
			/>,
		);
		expect(screen.getByText("active one")).toBeInTheDocument();
		expect(screen.getByText("resolved one")).toBeInTheDocument();
	});

	it("should show toggle button with resolved count", () => {
		render(
			<CommentList
				threads={[
					makeThread({ id: "t-1", resolved: true }),
					makeThread({ id: "t-2", resolved: true, lineNumber: 20 }),
				]}
				showResolvedThreads={false}
				onToggleShowResolved={vi.fn()}
			/>,
		);
		expect(screen.getByTestId("toggle-resolved-comments")).toBeInTheDocument();
		expect(screen.getByText(/Resolved \(2\)/)).toBeInTheDocument();
	});

	it("should call onToggleShowResolved when toggle button is clicked", async () => {
		const user = userEvent.setup();
		const onToggle = vi.fn();
		render(
			<CommentList
				threads={[makeThread({ id: "t-1", resolved: true })]}
				showResolvedThreads={false}
				onToggleShowResolved={onToggle}
			/>,
		);
		await user.click(screen.getByTestId("toggle-resolved-comments"));
		expect(onToggle).toHaveBeenCalledTimes(1);
	});

	it("should not show toggle button when no resolved comments", () => {
		render(
			<CommentList
				threads={[makeThread({ id: "t-1" })]}
				showResolvedThreads={false}
				onToggleShowResolved={vi.fn()}
			/>,
		);
		expect(
			screen.queryByTestId("toggle-resolved-comments"),
		).not.toBeInTheDocument();
	});

	it("should call onDeleteThread when delete button is clicked", async () => {
		const user = userEvent.setup();
		const onDelete = vi.fn();
		render(
			<CommentList
				threads={[
					makeThread({
						id: "t-42",
						entries: [
							{
								id: "e-1",
								content: "delete me",
								createdAt: Date.now(),
							},
						],
					}),
				]}
				onDeleteThread={onDelete}
			/>,
		);
		const row =
			screen
				.getByRole("button", { name: /delete me/i })
				.closest("[role='button']") ??
			screen.getByText("delete me").closest("[role='button']");
		if (!row) throw new Error("row not found");
		await user.hover(row);
		const deleteBtn = screen.getByLabelText("Delete thread");
		await user.click(deleteBtn);
		expect(onDelete).toHaveBeenCalledWith("t-42");
	});

	it("should call onResolveThread when resolve button is clicked", async () => {
		const user = userEvent.setup();
		const onResolve = vi.fn();
		render(
			<CommentList
				threads={[
					makeThread({
						id: "t-99",
						entries: [
							{
								id: "e-1",
								content: "resolve me",
								createdAt: Date.now(),
							},
						],
					}),
				]}
				onResolveThread={onResolve}
			/>,
		);
		const row = screen.getByText("resolve me").closest("[role='button']");
		if (!row) throw new Error("row not found");
		await user.hover(row);
		const resolveBtn = screen.getByLabelText("Resolve thread");
		await user.click(resolveBtn);
		expect(onResolve).toHaveBeenCalledWith("t-99");
	});

	it("should not show resolve button for already resolved threads", async () => {
		const user = userEvent.setup();
		render(
			<CommentList
				threads={[
					makeThread({
						id: "t-1",
						entries: [
							{
								id: "e-1",
								content: "already done",
								createdAt: Date.now(),
							},
						],
						resolved: true,
					}),
				]}
				showResolvedThreads={true}
				onResolveThread={vi.fn()}
				onDeleteThread={vi.fn()}
			/>,
		);
		const row = screen.getByText("already done").closest("[role='button']");
		if (!row) throw new Error("row not found");
		await user.hover(row);
		expect(screen.queryByLabelText("Resolve thread")).not.toBeInTheDocument();
		expect(screen.getByLabelText("Delete thread")).toBeInTheDocument();
	});

	it("should show navigation controls when unresolved threads exist", () => {
		render(
			<CommentList
				threads={[
					makeThread({ id: "t-1" }),
					makeThread({ id: "t-2", lineNumber: 20 }),
				]}
			/>,
		);
		expect(screen.getByText("2 unresolved")).toBeInTheDocument();
		expect(
			screen.getByLabelText("Previous unresolved thread"),
		).toBeInTheDocument();
		expect(screen.getByLabelText("Next unresolved thread")).toBeInTheDocument();
	});

	it("should not show navigation when no unresolved threads", () => {
		render(
			<CommentList
				threads={[makeThread({ id: "t-1", resolved: true })]}
				showResolvedThreads={true}
				onToggleShowResolved={vi.fn()}
			/>,
		);
		expect(screen.queryByText(/unresolved/)).not.toBeInTheDocument();
	});

	it("should call onThreadClick when navigating with next button", async () => {
		const user = userEvent.setup();
		const onClick = vi.fn();
		render(
			<CommentList
				threads={[
					makeThread({ id: "t-1", lineNumber: 10 }),
					makeThread({ id: "t-2", lineNumber: 20 }),
				]}
				onThreadClick={onClick}
			/>,
		);
		await user.click(screen.getByLabelText("Next unresolved thread"));
		expect(onClick).toHaveBeenCalledWith("/src/App.tsx", 10);
	});

	it("should not show action buttons when callbacks are not provided", () => {
		render(
			<CommentList
				threads={[
					makeThread({
						id: "t-1",
						entries: [
							{
								id: "e-1",
								content: "no actions",
								createdAt: Date.now(),
							},
						],
					}),
				]}
			/>,
		);
		expect(screen.queryByLabelText("Delete thread")).not.toBeInTheDocument();
		expect(screen.queryByLabelText("Resolve thread")).not.toBeInTheDocument();
	});
});
