import { render, screen, waitFor } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { WorktreeBranch } from "@/types/git";
import { DeleteWorktreeDialog } from "./DeleteWorktreeDialog";

const baseBranch: WorktreeBranch = {
	name: "feature/test",
	is_main_worktree: false,
	worktree_path: "/tmp/worktree/feature-test",
	dirty_count: 0,
	is_merged: false,
	has_pr: false,
	pr_number: null,
	pr_url: null,
	ahead: 0,
	behind: 0,
	has_upstream: true,
	base_ahead: 0,
};

const dirtyBranch: WorktreeBranch = {
	...baseBranch,
	dirty_count: 3,
};

describe("DeleteWorktreeDialog", () => {
	it("should show spinner and 'Deleting...' text when delete is in progress", async () => {
		const user = userEvent.setup();
		const onConfirm = vi.fn(
			() => new Promise<void>(() => {}), // never resolves
		);
		const onCancel = vi.fn();

		render(
			<DeleteWorktreeDialog
				open={true}
				branch={baseBranch}
				onConfirm={onConfirm}
				onCancel={onCancel}
			/>,
		);

		const deleteButton = screen.getByRole("button", { name: "Delete" });
		await user.click(deleteButton);

		await waitFor(() => {
			expect(screen.getByText("Deleting...")).toBeInTheDocument();
		});
		expect(deleteButton.querySelector(".animate-spin")).toBeInTheDocument();
		expect(deleteButton).toBeDisabled();
	});

	it("should show spinner on Force Delete button for dirty worktree", async () => {
		const user = userEvent.setup();
		const onConfirm = vi.fn(() => new Promise<void>(() => {}));
		const onCancel = vi.fn();

		render(
			<DeleteWorktreeDialog
				open={true}
				branch={dirtyBranch}
				onConfirm={onConfirm}
				onCancel={onCancel}
			/>,
		);

		const forceDeleteButton = screen.getByRole("button", {
			name: "Force Delete",
		});
		await user.click(forceDeleteButton);

		await waitFor(() => {
			expect(screen.getByText("Deleting...")).toBeInTheDocument();
		});
		expect(
			forceDeleteButton.querySelector(".animate-spin"),
		).toBeInTheDocument();
		expect(forceDeleteButton).toBeDisabled();
	});

	it("should not call onCancel while deleting", async () => {
		const user = userEvent.setup();
		const onConfirm = vi.fn(() => new Promise<void>(() => {}));
		const onCancel = vi.fn();

		render(
			<DeleteWorktreeDialog
				open={true}
				branch={baseBranch}
				onConfirm={onConfirm}
				onCancel={onCancel}
			/>,
		);

		const deleteButton = screen.getByRole("button", { name: "Delete" });
		await user.click(deleteButton);

		await waitFor(() => {
			expect(screen.getByText("Deleting...")).toBeInTheDocument();
		});

		// Cancel button should be disabled during deletion
		const cancelButton = screen.getByRole("button", { name: "Cancel" });
		expect(cancelButton).toBeDisabled();
		expect(onCancel).not.toHaveBeenCalled();
	});

	it("should reset deleting state after successful deletion so next delete works", async () => {
		const user = userEvent.setup();
		const onConfirm = vi.fn(() => Promise.resolve());
		const onCancel = vi.fn();

		const { rerender } = render(
			<DeleteWorktreeDialog
				open={true}
				branch={baseBranch}
				onConfirm={onConfirm}
				onCancel={onCancel}
			/>,
		);

		// First deletion
		const deleteButton = screen.getByRole("button", { name: "Delete" });
		await user.click(deleteButton);

		await waitFor(() => {
			expect(onConfirm).toHaveBeenCalledTimes(1);
		});

		// After success, button should not be stuck in "Deleting..." state
		expect(screen.queryByText("Deleting...")).not.toBeInTheDocument();

		// Simulate opening dialog for a different branch
		const anotherBranch: WorktreeBranch = {
			...baseBranch,
			name: "feature/other",
		};

		rerender(
			<DeleteWorktreeDialog
				open={true}
				branch={anotherBranch}
				onConfirm={onConfirm}
				onCancel={onCancel}
			/>,
		);

		// The delete button should be enabled and show "Delete", not "Deleting..."
		const nextDeleteButton = screen.getByRole("button", { name: "Delete" });
		expect(nextDeleteButton).not.toBeDisabled();
		expect(screen.queryByText("Deleting...")).not.toBeInTheDocument();

		// Second deletion should work
		await user.click(nextDeleteButton);

		await waitFor(() => {
			expect(onConfirm).toHaveBeenCalledTimes(2);
		});
	});

	it("should hide spinner and show error on failure", async () => {
		const user = userEvent.setup();
		const onConfirm = vi.fn(() => Promise.reject(new Error("Delete failed")));
		const onCancel = vi.fn();

		render(
			<DeleteWorktreeDialog
				open={true}
				branch={baseBranch}
				onConfirm={onConfirm}
				onCancel={onCancel}
			/>,
		);

		const deleteButton = screen.getByRole("button", { name: "Delete" });
		await user.click(deleteButton);

		await waitFor(() => {
			expect(screen.getByText("Error: Delete failed")).toBeInTheDocument();
		});

		// Spinner should be gone, button should be re-enabled
		expect(deleteButton.querySelector(".animate-spin")).toBeNull();
		expect(deleteButton).not.toBeDisabled();
	});
});
