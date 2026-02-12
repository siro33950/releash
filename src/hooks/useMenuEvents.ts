import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef } from "react";

export type MenuEventId =
	| "new-file"
	| "new-folder"
	| "open-folder"
	| "save"
	| "save-all"
	| "close-tab"
	| "close-all-tabs"
	| "find-in-files"
	| "view-explorer"
	| "view-search"
	| "view-source-control"
	| "diff-gutter"
	| "diff-inline"
	| "diff-split"
	| "theme-dark"
	| "theme-light"
	| "increase-font-size"
	| "decrease-font-size"
	| "reset-font-size"
	| "git-stage-all"
	| "git-unstage-all"
	| "git-commit"
	| "git-push"
	| "git-discard-all"
	| "git-create-branch"
	| "new-terminal"
	| "back-to-kanban"
	| "remote-start-server"
	| "remote-stop-server"
	| "remote-show-qr"
	| "settings";

export type MenuHandlers = Partial<Record<MenuEventId, () => void>>;

export function useMenuEvents(handlers: MenuHandlers, enabled = true) {
	const handlersRef = useRef(handlers);
	handlersRef.current = handlers;
	const enabledRef = useRef(enabled);
	enabledRef.current = enabled;

	useEffect(() => {
		const unlisten = listen<string>("menu-event", (event) => {
			if (!enabledRef.current) return;
			const handler = handlersRef.current[event.payload as MenuEventId];
			if (handler) {
				handler();
			}
		});
		return () => {
			unlisten.then((f) => f());
		};
	}, []);
}
