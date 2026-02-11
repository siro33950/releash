interface UseKeyboardShortcutsOptions {
	onSave?: () => void;
	onSearch?: () => void;
}

export function useKeyboardShortcuts(_options: UseKeyboardShortcutsOptions) {
	// Cmd+S and Cmd+Shift+F are now handled by native menu accelerators.
	// This hook is kept for API compatibility but is intentionally a no-op.
}
