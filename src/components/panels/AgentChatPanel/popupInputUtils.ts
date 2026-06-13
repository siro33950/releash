/**
 * UI input utilities for autocomplete popup trigger detection and keyboard navigation.
 *
 * These functions handle cursor-position-based UI control (when to show/hide popups
 * and how to navigate them via keyboard). They are placed in the frontend because
 * they operate on DOM-level input state (selectionStart, keyDown events) and fall
 * under "user input handling" — not business logic.
 */

/**
 * Find the position of the `@` trigger for mention autocomplete in text.
 * Scans backwards from the cursor to detect a valid `@` trigger:
 * - `@` must be at the start of text or preceded by whitespace
 * - Unquoted query text between `@` and cursor must not contain whitespace
 * - Quoted query text after `@"` may contain whitespace until the closing quote
 */
export function findMentionTrigger(
	text: string,
	cursorPos: number,
): { start: number; query: string } | null {
	for (let i = cursorPos - 1; i >= 0; i--) {
		const ch = text[i];
		if (ch === "@") {
			if (i !== 0 && !/\s/.test(text[i - 1])) return null;
			if (text[i + 1] === '"') {
				const query = text.slice(i + 2, cursorPos);
				if (hasUnescapedQuote(query)) return null;
				return { start: i, query: unescapeQuotedMentionQuery(query) };
			}
			const query = text.slice(i + 1, cursorPos);
			if (!/\s/.test(query)) {
				return { start: i, query };
			}
			return null;
		}
	}
	return null;
}

export function findSkillTrigger(
	text: string,
	cursorPos: number,
): { start: number; query: string } | null {
	return findTokenTrigger(text, cursorPos, "$");
}

function findTokenTrigger(
	text: string,
	cursorPos: number,
	triggerChar: "@" | "$",
): { start: number; query: string } | null {
	for (let i = cursorPos - 1; i >= 0; i--) {
		const ch = text[i];
		if (ch === triggerChar) {
			if (i === 0 || /\s/.test(text[i - 1])) {
				const query = text.slice(i + 1, cursorPos);
				if (!/\s/.test(query)) {
					return { start: i, query };
				}
			}
			return null;
		}
		if (/\s/.test(ch)) {
			return null;
		}
	}
	return null;
}

function hasUnescapedQuote(value: string): boolean {
	let escaped = false;
	for (const ch of value) {
		if (escaped) {
			escaped = false;
			continue;
		}
		if (ch === "\\") {
			escaped = true;
			continue;
		}
		if (ch === '"') return true;
	}
	return false;
}

function unescapeQuotedMentionQuery(value: string): string {
	let result = "";
	let escaped = false;
	for (const ch of value) {
		if (escaped) {
			result += ch;
			escaped = false;
			continue;
		}
		if (ch === "\\") {
			escaped = true;
			continue;
		}
		result += ch;
	}
	if (escaped) result += "\\";
	return result;
}

/**
 * Handle keyboard navigation for a popup list (ArrowDown/Up, Enter, Tab, Escape).
 * Returns true if the event was consumed by the popup.
 */
export function handlePopupKeyDown(
	e: React.KeyboardEvent,
	itemCount: number,
	setSelectedIndex: React.Dispatch<React.SetStateAction<number>>,
	onSelect: () => void,
	onDismiss: () => void,
): boolean {
	if (e.key === "ArrowDown") {
		e.preventDefault();
		setSelectedIndex((i) => (i >= itemCount - 1 ? 0 : i + 1));
		return true;
	}
	if (e.key === "ArrowUp") {
		e.preventDefault();
		setSelectedIndex((i) => (i <= 0 ? itemCount - 1 : i - 1));
		return true;
	}
	if (e.key === "Enter" && !e.metaKey && !e.ctrlKey) {
		e.preventDefault();
		onSelect();
		return true;
	}
	if (e.key === "Tab" && !e.shiftKey) {
		e.preventDefault();
		onSelect();
		return true;
	}
	if (e.key === "Escape") {
		e.preventDefault();
		onDismiss();
		return true;
	}
	return false;
}
