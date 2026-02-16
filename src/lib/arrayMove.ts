export function arrayMove<T>(arr: readonly T[], from: number, to: number): T[] {
	if (from === to) return [...arr];
	if (from < 0 || from >= arr.length || to < 0 || to >= arr.length) {
		return [...arr];
	}
	const result = [...arr];
	const [item] = result.splice(from, 1);
	result.splice(to, 0, item);
	return result;
}
