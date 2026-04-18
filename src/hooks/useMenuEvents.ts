import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef } from "react";

export type MenuEventId =
	| "open-folder"
	| "theme-dark"
	| "theme-light"
	| "increase-font-size"
	| "decrease-font-size"
	| "reset-font-size"
	| "git-stage-all"
	| "git-unstage-all"
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
