import { render, screen, waitFor } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { BranchCard } from "@/types/git";
import { DeleteWorktreeDialog } from "./DeleteWorktreeDialog";

const baseBranch: BranchCard = {
	name: "feature/test",
	is_default: false,
	worktree_path: "/tmp/worktree/feature-test",
	dirty_count: 0,
	is_merged: false,
	has_pr: false,
	pr_number: null,
	pr_url: null,
	ahead: 0,
	behind: 0,
	is_remote_only: false,
	has_upstream: true,
	remote_name: null,
};

const dirtyBranch: BranchCard = {
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
