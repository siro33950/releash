import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { WorktreeErrorBoundary } from "./ErrorBoundary";

function ThrowingChild({ shouldThrow }: { shouldThrow: boolean }) {
	if (shouldThrow) {
		throw new Error("test render error");
	}
	return <div>normal content</div>;
}

describe("WorktreeErrorBoundary", () => {
	it("renders children when no error occurs", () => {
		render(
			<WorktreeErrorBoundary>
				<ThrowingChild shouldThrow={false} />
			</WorktreeErrorBoundary>,
		);
		expect(screen.getByText("normal content")).toBeInTheDocument();
	});

	it("renders fallback UI when child throws", () => {
		const consoleSpy = vi.spyOn(console, "error").mockImplementation(() => {});

		render(
			<WorktreeErrorBoundary>
				<ThrowingChild shouldThrow={true} />
			</WorktreeErrorBoundary>,
		);

		expect(
			screen.getByText("ビューの描画中にエラーが発生しました"),
		).toBeInTheDocument();
		expect(screen.getByText("test render error")).toBeInTheDocument();
		expect(screen.getByRole("button", { name: "再試行" })).toBeInTheDocument();

		consoleSpy.mockRestore();
	});

	it("calls onRetry and resets state when retry button is clicked", async () => {
		const consoleSpy = vi.spyOn(console, "error").mockImplementation(() => {});
		const onRetry = vi.fn();
		const user = userEvent.setup();

		const { rerender } = render(
			<WorktreeErrorBoundary onRetry={onRetry}>
				<ThrowingChild shouldThrow={true} />
			</WorktreeErrorBoundary>,
		);

		expect(
			screen.getByText("ビューの描画中にエラーが発生しました"),
		).toBeInTheDocument();

		rerender(
			<WorktreeErrorBoundary onRetry={onRetry}>
				<ThrowingChild shouldThrow={false} />
			</WorktreeErrorBoundary>,
		);

		await user.click(screen.getByRole("button", { name: "再試行" }));

		expect(onRetry).toHaveBeenCalledOnce();
		expect(screen.getByText("normal content")).toBeInTheDocument();

		consoleSpy.mockRestore();
	});
});
