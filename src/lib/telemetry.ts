import { invoke } from "@tauri-apps/api/core";

let mountedXtermCount = 0;
let errorHandlersInstalled = false;
let errorHandler: ((event: ErrorEvent) => void) | null = null;
let rejectionHandler: ((event: PromiseRejectionEvent) => void) | null = null;

function invokeTelemetry(
	command: string,
	args?: Record<string, unknown>,
): Promise<void> {
	return invoke(command, args)
		.then(() => undefined)
		.catch(() => {});
}

export function setPerformanceTelemetryEnabled(
	enabled: boolean,
): Promise<void> {
	return invokeTelemetry("update_performance_telemetry", { enabled });
}

export function trackEvent(name: string): void {
	invokeTelemetry("report_usage_event", { name });
}

export function reportMountedXtermMounted(): void {
	mountedXtermCount += 1;
	invokeTelemetry("report_mounted_xterm_count", { count: mountedXtermCount });
}

export function reportMountedXtermUnmounted(): void {
	mountedXtermCount = Math.max(0, mountedXtermCount - 1);
	invokeTelemetry("report_mounted_xterm_count", { count: mountedXtermCount });
}

function normalizeError(error: unknown): { message: string; stack?: string } {
	if (error instanceof Error) {
		return { message: error.message, stack: error.stack };
	}
	if (typeof error === "string") {
		return { message: error };
	}
	return { message: String(error) };
}

export function reportFrontendError(
	error: unknown,
	errorType = "frontend_error",
	componentStack?: string,
): void {
	const normalized = normalizeError(error);
	const stack = [normalized.stack, componentStack].filter(Boolean).join("\n");
	invokeTelemetry("report_frontend_error", {
		payload: {
			errorType,
			message: normalized.message,
			stack: stack.length > 0 ? stack : undefined,
		},
	});
}

export function installFrontendErrorHandlers(): void {
	if (errorHandlersInstalled || typeof window === "undefined") return;
	errorHandlersInstalled = true;
	errorHandler = (event) => {
		reportFrontendError(event.error ?? event.message, "unhandled_error");
	};
	rejectionHandler = (event) => {
		reportFrontendError(event.reason, "unhandled_rejection");
	};
	window.addEventListener("error", errorHandler);
	window.addEventListener("unhandledrejection", rejectionHandler);
}

export function __resetTelemetryForTests(): void {
	if (typeof window !== "undefined") {
		if (errorHandler) window.removeEventListener("error", errorHandler);
		if (rejectionHandler) {
			window.removeEventListener("unhandledrejection", rejectionHandler);
		}
	}
	mountedXtermCount = 0;
	errorHandlersInstalled = false;
	errorHandler = null;
	rejectionHandler = null;
}
