import { useEffect, useState } from "react";
import { onClientRefresh } from "@/lib/clientSocket";

export function useClientRefresh(enabled = true) {
	const [controller, setController] = useState(() => new AbortController());
	useEffect(() => {
		if (!enabled) return;
		return onClientRefresh(() => {
			controller.abort();
			setController(new AbortController());
		});
	}, [enabled, controller]);
	return controller.signal;
}
