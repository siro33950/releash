import type { Thread } from "@/types/thread";

export function formatCommentsForTerminal(
	threads: Thread[],
	rootPath?: string,
): string {
	if (threads.length === 0) return "";

	const grouped = new Map<string, Thread[]>();
	for (const thread of threads) {
		const existing = grouped.get(thread.filePath);
		if (existing) {
			existing.push(thread);
		} else {
			grouped.set(thread.filePath, [thread]);
		}
	}

	const blocks: string[] = [];
	for (const [filePath, fileThreads] of grouped) {
		const prefix = rootPath ? `${rootPath}/` : "";
		const relativePath = filePath.startsWith(prefix)
			? filePath.slice(prefix.length)
			: filePath;
		const sorted = [...fileThreads].sort((a, b) => a.lineNumber - b.lineNumber);
		for (const t of sorted) {
			const lineLabel =
				t.endLine != null
					? `L${t.lineNumber}-${t.endLine}`
					: `L${t.lineNumber}`;
			const content = t.entries.map((e) => e.content).join("\n---\n");
			blocks.push(`${relativePath}:${lineLabel}\n${content}`);
		}
	}

	return `## Review Comments\n${blocks.join("\n=====\n")}`;
}
