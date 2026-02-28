export function formatRelativeTime(createdAt: number): string {
	const diff = Date.now() - createdAt;
	if (diff < 60000) return "now";
	if (diff < 3600000) return `${Math.floor(diff / 60000)}m`;
	if (diff < 86400000) return `${Math.floor(diff / 3600000)}h`;
	return `${Math.floor(diff / 86400000)}d`;
}
