import { beforeEach, describe, expect, it, vi } from "vitest";
import { invokeClient as invoke } from "@/lib/clientSocket";
import { FrontendErrorBoundary } from "./ErrorBoundary";

describe("FrontendErrorBoundary", () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	it("componentDidCatch から react_error をRustへ送る", () => {
		const boundary = new FrontendErrorBoundary({ children: null });

		boundary.componentDidCatch(new Error("render boom"), {
			componentStack: "\n    at BrokenComponent",
		});

		expect(invoke).toHaveBeenCalledWith("report_frontend_error", {
			payload: expect.objectContaining({
				errorType: "react_error",
				message: "render boom",
				stack: expect.stringContaining("BrokenComponent"),
			}),
		});
	});
});
