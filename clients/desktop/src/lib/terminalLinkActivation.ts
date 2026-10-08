import { openUrl } from "@tauri-apps/plugin-opener";
import { logClientError } from "@/lib/clientErrorNotice";

export function activateTerminalLink(url: string): void {
	void openUrl(url).catch((error: unknown) => {
		logClientError("Failed to open terminal link:", error);
	});
}
