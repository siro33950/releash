import { getErrorMessage } from "@/lib/errorMessage";

export function showClientError(error: unknown) {
	window.dispatchEvent(
		new CustomEvent<string>("releash-client-error", {
			detail: getErrorMessage(error),
		}),
	);
}
