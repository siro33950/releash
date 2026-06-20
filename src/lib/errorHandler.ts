export interface ErrorContext {
	operation?: string;
	componentName?: string;
}

export function formatUserFriendlyError(
	error: unknown,
	context?: ErrorContext,
): string {
	const errorMessage = String(error);
	const lowerMessage = errorMessage.toLowerCase();

	// TypeError: null reading
	if (errorMessage.includes("Cannot read properties of null")) {
		if (context?.operation) {
			return `Failed to ${context.operation}. Please try again.`;
		}
		return "An unexpected error occurred. Please try again.";
	}

	// TypeError: undefined reading
	if (errorMessage.includes("Cannot read properties of undefined")) {
		return "Data is not ready yet. Please wait a moment.";
	}

	// Network errors - more specific patterns to avoid false positives
	if (
		lowerMessage.includes("network error") ||
		lowerMessage.includes("failed to fetch") ||
		lowerMessage.includes("network request failed")
	) {
		return "Network connection error. Please check your connection and try again.";
	}

	// Tauri command errors
	if (errorMessage.includes("command") && errorMessage.includes("not found")) {
		return "System command not available. Please restart the app.";
	}

	// Git errors - pass through with prefix
	if (lowerMessage.includes("git")) {
		return `Git operation failed: ${errorMessage}`;
	}

	// Dev mode: log original error
	if (import.meta.env.DEV) {
		console.error("Original error:", error, "Context:", context);
	}

	// Default: show message if short, otherwise generic
	return errorMessage.length > 150
		? "An error occurred. Please check the console for details."
		: errorMessage;
}

export function formatGitError(error: unknown): string {
	return formatUserFriendlyError(error, {
		operation: "execute git command",
		componentName: "SourceControlPanel",
	});
}
