import { listen } from "@tauri-apps/api/event";
import { useEffect } from "react";

export type MenuHandlers = Partial<Record<string, () => void>>;

export function useMenuEvents(handlers: MenuHandlers) {
	useEffect(() => {
		const unlisten = listen<string>("menu-event", (event) => {
			const handler = handlers[event.payload];
			if (handler) {
				handler();
			}
		});
		return () => {
			unlisten.then((f) => f());
		};
	}, [handlers]);
}
