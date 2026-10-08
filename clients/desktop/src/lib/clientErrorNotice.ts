import { getErrorMessage } from "@/lib/errorMessage";

export function showClientError(error: unknown) {
	window.dispatchEvent(
		new CustomEvent<string>("releash-client-error", {
			detail: getErrorMessage(error),
		}),
	);
}

export function rethrowClientError(error: unknown): never {
	showClientError(error);
	throw error;
}

export function logClientError(context: string, error: unknown) {
	console.error(context, error);
	showClientError(error);
}
