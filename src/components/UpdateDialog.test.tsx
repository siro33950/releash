import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { UpdateCheckResult } from "@/hooks/useUpdateChecker";
import { UpdateDialog } from "./UpdateDialog";

function makeUpdate(
	overrides: Partial<UpdateCheckResult> = {},
): UpdateCheckResult {
	return {
		status: "idle",
		updateInfo: null,
		progress: 0,
		error: null,
		downloadAndInstall: vi.fn(),
		dismiss: vi.fn(),
		...overrides,
	};
}

describe("UpdateDialog", () => {
	it("should not render when idle", () => {
		const { container } = render(<UpdateDialog update={makeUpdate()} />);
		expect(container.innerHTML).toBe("");
	});

	it("should not render when checking", () => {
		const { container } = render(
			<UpdateDialog update={makeUpdate({ status: "checking" })} />,
		);
		expect(container.innerHTML).toBe("");
	});

	it("should show version and buttons when available", () => {
		render(
			<UpdateDialog
				update={makeUpdate({
					status: "available",
					updateInfo: { version: "2.0.0", notes: "New feature" },
				})}
			/>,
		);
		expect(screen.getByText("Update Available")).toBeInTheDocument();
		expect(screen.getByText(/2\.0\.0/)).toBeInTheDocument();
		expect(screen.getByText("New feature")).toBeInTheDocument();
		expect(screen.getByText("Later")).toBeInTheDocument();
		expect(screen.getByText("Update Now")).toBeInTheDocument();
	});

	it("should call dismiss on Later click", () => {
		const dismiss = vi.fn();
		render(
			<UpdateDialog
				update={makeUpdate({
					status: "available",
					updateInfo: { version: "2.0.0", notes: "" },
					dismiss,
				})}
			/>,
		);
		fireEvent.click(screen.getByText("Later"));
		expect(dismiss).toHaveBeenCalled();
	});

	it("should call downloadAndInstall on Update Now click", () => {
		const downloadAndInstall = vi.fn();
		render(
			<UpdateDialog
				update={makeUpdate({
					status: "available",
					updateInfo: { version: "2.0.0", notes: "" },
					downloadAndInstall,
				})}
			/>,
		);
		fireEvent.click(screen.getByText("Update Now"));
		expect(downloadAndInstall).toHaveBeenCalled();
	});

	it("should show progress bar when downloading", () => {
		render(
			<UpdateDialog
				update={makeUpdate({
					status: "downloading",
					progress: 45,
				})}
			/>,
		);
		expect(screen.getByText("Downloading Update...")).toBeInTheDocument();
		expect(screen.getByRole("progressbar")).toHaveAttribute(
			"aria-valuenow",
			"45",
		);
		expect(screen.getByText("45%")).toBeInTheDocument();
	});

	it("should call dismiss on Cancel click during downloading", () => {
		const dismiss = vi.fn();
		render(
			<UpdateDialog
				update={makeUpdate({
					status: "downloading",
					progress: 30,
					dismiss,
				})}
			/>,
		);
		fireEvent.click(screen.getByText("Cancel"));
		expect(dismiss).toHaveBeenCalled();
	});

	it("should show error message when error", () => {
		const dismiss = vi.fn();
		render(
			<UpdateDialog
				update={makeUpdate({
					status: "error",
					error: "Something went wrong",
					dismiss,
				})}
			/>,
		);
		expect(screen.getByText("Update Error")).toBeInTheDocument();
		expect(screen.getByText("Something went wrong")).toBeInTheDocument();
		fireEvent.click(screen.getByText("Close"));
		expect(dismiss).toHaveBeenCalled();
	});
});
