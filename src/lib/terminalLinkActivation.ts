import { openUrl } from "@tauri-apps/plugin-opener";

export function activateTerminalLink(url: string): void {
	void openUrl(url).catch((error: unknown) => {
		console.error("Failed to open terminal link:", error);
	});
}
