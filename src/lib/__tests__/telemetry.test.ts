import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
	installFrontendErrorHandlers,
	reportFrontendError,
	reportMountedXtermMounted,
	reportMountedXtermUnmounted,
	setPerformanceTelemetryEnabled,
	trackEvent,
} from "../telemetry";

describe("telemetry", () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	it("enabled の場合、Rust の usage event コマンドを呼び出す", () => {
		trackEvent("test_event");
		expect(invoke).toHaveBeenCalledWith("report_usage_event", {
			name: "test_event",
		});
	});

	it("performance telemetry disabled でも usage event 転送は frontend でゲートしない", async () => {
		await setPerformanceTelemetryEnabled(false);
		vi.clearAllMocks();
		trackEvent("test_event");
		expect(invoke).toHaveBeenCalledWith("report_usage_event", {
			name: "test_event",
		});
	});

	it("performance telemetry 更新は Rust command を呼び出す", async () => {
		await setPerformanceTelemetryEnabled(false);
		trackEvent("first");
		expect(invoke).toHaveBeenCalledWith("update_performance_telemetry", {
			enabled: false,
		});

		await setPerformanceTelemetryEnabled(true);
		trackEvent("second");
		expect(invoke).toHaveBeenCalledWith("report_usage_event", {
			name: "second",
		});
	});

	it("xterm の mount 数をRustへ送る", () => {
		reportMountedXtermMounted();
		reportMountedXtermMounted();
		reportMountedXtermUnmounted();
		expect(invoke).toHaveBeenCalledWith("report_mounted_xterm_count", {
			count: 1,
		});
	});

	it("xterm の unmount は 0 未満にならない", () => {
		reportMountedXtermUnmounted();
		expect(invoke).toHaveBeenCalledWith("report_mounted_xterm_count", {
			count: 0,
		});
	});

	it("xterm の mount/unmount は対称に増減する", () => {
		reportMountedXtermMounted();
		reportMountedXtermUnmounted();
		expect(invoke).toHaveBeenNthCalledWith(1, "report_mounted_xterm_count", {
			count: 1,
		});
		expect(invoke).toHaveBeenNthCalledWith(2, "report_mounted_xterm_count", {
			count: 0,
		});
	});

	it("frontend error をRustへ送る", () => {
		const error = new Error("boom");
		error.stack = "error stack";
		reportFrontendError(error, "react_error", "component stack");
		expect(invoke).toHaveBeenCalledWith("report_frontend_error", {
			payload: expect.objectContaining({
				errorType: "react_error",
				message: "boom",
				stack: "error stack\ncomponent stack",
			}),
		});
	});

	it("プレーン文字列のfrontend errorをそのままRustへ送る", () => {
		reportFrontendError("plain failure");

		expect(invoke).toHaveBeenCalledWith("report_frontend_error", {
			payload: {
				errorType: "frontend_error",
				message: "plain failure",
				stack: undefined,
			},
		});
	});

	it("window error から frontend error をRustへ送る", () => {
		installFrontendErrorHandlers();
		window.dispatchEvent(
			new ErrorEvent("error", {
				error: new Error("window boom"),
				message: "window boom",
			}),
		);
		expect(invoke).toHaveBeenCalledWith("report_frontend_error", {
			payload: expect.objectContaining({
				errorType: "unhandled_error",
				message: "window boom",
			}),
		});
	});

	it("unhandledrejection から frontend error をRustへ送る", () => {
		installFrontendErrorHandlers();
		const event = new Event("unhandledrejection") as PromiseRejectionEvent;
		Object.defineProperty(event, "reason", {
			value: {
				code: "AGENT_SESSION_LAUNCH_UNAVAILABLE",
				message: "coded promise failure",
			},
		});
		window.dispatchEvent(event);
		expect(invoke).toHaveBeenCalledWith("report_frontend_error", {
			payload: expect.objectContaining({
				errorType: "unhandled_rejection",
				message: "coded promise failure",
			}),
		});
	});

	it("type tagged errorのmessageをunhandledrejectionからRustへ送る", () => {
		installFrontendErrorHandlers();
		const event = new Event("unhandledrejection") as PromiseRejectionEvent;
		Object.defineProperty(event, "reason", {
			value: {
				type: "invalid_request",
				message:
					"Releash could not start the application quit because the request is invalid.",
			},
		});
		window.dispatchEvent(event);
		expect(invoke).toHaveBeenCalledWith("report_frontend_error", {
			payload: expect.objectContaining({
				errorType: "unhandled_rejection",
				message:
					"Releash could not start the application quit because the request is invalid.",
			}),
		});
	});

	it("Rust コマンドがエラーを返してもクラッシュしない", () => {
		vi.mocked(invoke).mockRejectedValueOnce(new Error("ipc error"));
		expect(() => trackEvent("fail_event")).not.toThrow();
	});
});
