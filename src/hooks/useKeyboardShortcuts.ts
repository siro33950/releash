import { useEffect } from "react";

interface UseKeyboardShortcutsOptions {
	onSave?: () => void;
}

export function useKeyboardShortcuts({ onSave }: UseKeyboardShortcutsOptions) {
	useEffect(() => {
		const handleKeyDown = (e: KeyboardEvent) => {
			if ((e.metaKey || e.ctrlKey) && e.key === "s") {
				e.preventDefault();
				onSave?.();
			}
		};

		window.addEventListener("keydown", handleKeyDown);
		return () => window.removeEventListener("keydown", handleKeyDown);
	}, [onSave]);
}
