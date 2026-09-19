import { ConnectError } from "@connectrpc/connect";

export function getErrorMessage(error: unknown): string {
	if (error instanceof ConnectError) return "処理中にエラーが発生しました";
	if (error instanceof Error) return error.message;
	if (
		typeof error === "object" &&
		error !== null &&
		"message" in error &&
		typeof error.message === "string"
	) {
		return error.message;
	}
	return String(error);
}
