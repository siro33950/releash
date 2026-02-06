import { useEffect } from "react";

interface UseKeyboardShortcutsOptions {
	onSave?: () => void;
	onSearch?: () => void;
}

export function useKeyboardShortcuts({
	onSave,
	onSearch,
}: UseKeyboardShortcutsOptions) {
	useEffect(() => {
		const handleKeyDown = (e: KeyboardEvent) => {
			if ((e.metaKey || e.ctrlKey) && e.key === "s") {
				e.preventDefault();
				onSave?.();
			}
			if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key === "f") {
				e.preventDefault();
				onSearch?.();
			}
		};

		window.addEventListener("keydown", handleKeyDown);
		return () => window.removeEventListener("keydown", handleKeyDown);
	}, [onSave, onSearch]);
}
