import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { GitFileStatus } from "@/types/git";
import { FileStatusItem, formatPath, statusColor } from "./FileStatusItem";

describe("statusColor", () => {
	it("should return untracked color for 'new'", () => {
		expect(statusColor("new")).toBe("text-status-untracked");
	});

	it("should return modified color for 'modified'", () => {
		expect(statusColor("modified")).toBe("text-status-modified");
	});

	it("should return deleted color for 'deleted'", () => {
		expect(statusColor("deleted")).toBe("text-status-deleted");
	});

	it("should return modified color for 'renamed'", () => {
		expect(statusColor("renamed")).toBe("text-status-modified");
	});

	it("should return muted foreground for unknown status", () => {
		expect(statusColor("unknown")).toBe("text-muted-foreground");
	});
});

describe("formatPath", () => {
	it("should return name only for file without directory", () => {
		expect(formatPath("file.txt")).toEqual({ dir: "", name: "file.txt" });
	});

	it("should split single-level directory", () => {
		expect(formatPath("src/file.txt")).toEqual({
			dir: "src/",
			name: "file.txt",
		});
	});

	it("should split multi-level directory", () => {
		expect(formatPath("src/components/panels/File.tsx")).toEqual({
			dir: "src/components/panels/",
			name: "File.tsx",
		});
	});
});

describe("FileStatusItem", () => {
	const baseEntry: GitFileStatus = {
		path: "src/file.txt",
		index_status: "none",
		worktree_status: "modified",
	};

	it("should render file name and directory", () => {
		render(
			<FileStatusItem
				entry={baseEntry}
				statusField="worktree_status"
				actionLabel="Stage"
				onAction={vi.fn()}
			/>,
		);
		expect(screen.getByText("file.txt")).toBeInTheDocument();
		expect(screen.getByText("src/")).toBeInTheDocument();
	});

	it("should apply selected style when selected is true", () => {
		const { container } = render(
			<FileStatusItem
				entry={baseEntry}
				statusField="worktree_status"
				selected={true}
				actionLabel="Stage"
				onAction={vi.fn()}
			/>,
		);
		const row = container.querySelector("[role='button']");
		expect(row?.className).toContain("bg-foreground/10");
		expect(row?.className).not.toContain("hover:bg-foreground/5");
	});

	it("should show action button always when alwaysShowAction is true", () => {
		render(
			<FileStatusItem
				entry={baseEntry}
				statusField="worktree_status"
				actionLabel="Stage"
				onAction={vi.fn()}
				alwaysShowAction
			/>,
		);
		const actionButton = screen.getByTitle("Stage");
		expect(actionButton.className).toContain("inline-flex");
		expect(actionButton.className).not.toContain("hidden");
	});

	it("should hide action button by default (show on hover)", () => {
		render(
			<FileStatusItem
				entry={baseEntry}
				statusField="worktree_status"
				actionLabel="Stage"
				onAction={vi.fn()}
			/>,
		);
		const actionButton = screen.getByTitle("Stage");
		expect(actionButton.className).toContain("hidden");
	});

	it("should call onSelect with entry when clicked", () => {
		const onSelect = vi.fn();
		render(
			<FileStatusItem
				entry={baseEntry}
				statusField="worktree_status"
				onSelect={onSelect}
				actionLabel="Stage"
				onAction={vi.fn()}
			/>,
		);
		fireEvent.click(screen.getByRole("button", { name: /file\.txt/ }));
		expect(onSelect).toHaveBeenCalledWith(baseEntry);
	});

	it("should call onSelect on Enter key", () => {
		const onSelect = vi.fn();
		render(
			<FileStatusItem
				entry={baseEntry}
				statusField="worktree_status"
				onSelect={onSelect}
				actionLabel="Stage"
				onAction={vi.fn()}
			/>,
		);
		const row = screen.getByRole("button", { name: /file\.txt/ });
		fireEvent.keyDown(row, { key: "Enter" });
		expect(onSelect).toHaveBeenCalledWith(baseEntry);
	});

	it("should call onAction when action button is clicked", () => {
		const onAction = vi.fn();
		render(
			<FileStatusItem
				entry={baseEntry}
				statusField="worktree_status"
				actionLabel="Stage"
				onAction={onAction}
				alwaysShowAction
			/>,
		);
		fireEvent.click(screen.getByTitle("Stage"));
		expect(onAction).toHaveBeenCalledOnce();
	});

	it("should not call onSelect when action button is clicked", () => {
		const onSelect = vi.fn();
		const onAction = vi.fn();
		render(
			<FileStatusItem
				entry={baseEntry}
				statusField="worktree_status"
				onSelect={onSelect}
				actionLabel="Stage"
				onAction={onAction}
				alwaysShowAction
			/>,
		);
		fireEvent.click(screen.getByTitle("Stage"));
		expect(onAction).toHaveBeenCalledOnce();
		expect(onSelect).not.toHaveBeenCalled();
	});
});
