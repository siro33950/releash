import type { FileNode, FileStatus } from "@/types/file-tree";

function computeFoldersWithChanges(
	statusMap: Map<string, FileStatus>,
): Set<string> {
	const result = new Set<string>();
	for (const [filePath, status] of statusMap) {
		if (status === "ignored") continue;
		let parent = filePath;
		while (true) {
			const slashIndex = parent.lastIndexOf("/");
			if (slashIndex === -1) break;
			parent = parent.substring(0, slashIndex);
			result.add(parent);
		}
	}
	return result;
}

function applyStatusRecursive(
	nodes: FileNode[],
	statusMap: Map<string, FileStatus>,
	foldersWithChanges: Set<string>,
	parentIgnored: boolean,
): FileNode[] {
	return nodes.map((node) => {
		if (node.type === "file") {
			const status =
				statusMap.get(node.path) ?? (parentIgnored ? "ignored" : null);
			if (status === (node.status ?? null)) return node;
			return { ...node, status };
		}

		const isIgnored =
			statusMap.get(node.path) === "ignored" || parentIgnored;

		const children = node.children
			? applyStatusRecursive(
					node.children,
					statusMap,
					foldersWithChanges,
					isIgnored,
				)
			: undefined;

		let status: FileStatus;
		if (isIgnored) {
			status = "ignored";
		} else if (children === undefined) {
			status = foldersWithChanges.has(node.path) ? "modified" : null;
		} else {
			const hasChangedChild = children.some(
				(child) => child.status != null && child.status !== "ignored",
			);
			status = hasChangedChild ? "modified" : null;
		}

		if (status === node.status && children === node.children) return node;
		return { ...node, status, children };
	});
}

export function applyStatusToTree(
	nodes: FileNode[],
	statusMap: Map<string, FileStatus>,
): FileNode[] {
	const foldersWithChanges = computeFoldersWithChanges(statusMap);
	return applyStatusRecursive(nodes, statusMap, foldersWithChanges, false);
}
